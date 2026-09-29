"""Offline Linux TCP backpressure through real scoped Vessel WebSockets.

Uses the existing disposable process/account fixture and synthetic provider only.
Run after building: python3 voyage/tests/socket_backpressure.py --bin-dir target/debug
"""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import socket
import struct
import threading
import time
from urllib.parse import urlsplit
import uuid

from approval_semantics import Gateway
from delivery_recovery import wait_for
from goals import Fixture

CHUNKS = 1200
TEXT = "stream-" + "x" * 504 + "\n"  # 512 bytes, below the run's 1 MiB cap.


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            length = int(self.headers["Content-Length"])
            assert 0 < length <= 4 * 1024 * 1024
            self.server.bodies.append(json.loads(self.rfile.read(length)))
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.server.entered.set()
            assert self.server.begin.wait(30), "subscriber setup timeout"
            for _ in range(CHUNKS):
                if self.server.abort.is_set():
                    return
                self.wfile.write(("data: " + json.dumps({"type": "response.output_text.delta", "delta": TEXT}) + "\n\n").encode())
                self.wfile.flush()
                time.sleep(0.025)
            self.server.sent.set()
            assert self.server.finish.wait(90), "recovery observation timeout"
            response = {"id": "fixture-backpressure", "status": "completed", "output": [
                {"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": TEXT * CHUNKS}]}],
                "usage": {"input_tokens": 1, "output_tokens": 1}}
            self.wfile.write(("data: " + json.dumps({"type": "response.completed", "response": response}) + "\n\n").encode())
            self.wfile.flush()
        except Exception as error:
            self.server.errors.append(str(error))


class WebSocket:
    """Small fixture-only RFC6455 client; production client implementation is unchanged."""
    def __init__(self, endpoint, credential, slow=False):
        address = urlsplit(endpoint)
        self.socket = socket.socket()
        try:
            if slow:
                self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
                self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_WINDOW_CLAMP, 1024)
            self.receive_buffer = self.socket.getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF)
            self.socket.settimeout(10)
            self.socket.connect((address.hostname, address.port))
            self.port = self.socket.getsockname()[1]
            self.lock = threading.Lock()
            key = base64.b64encode(os.urandom(16)).decode()
            headers = {"Host": address.netloc, "Connection": "Upgrade", "Upgrade": "websocket",
                "Sec-WebSocket-Version": "13", "Sec-WebSocket-Key": key,
                "Sec-WebSocket-Protocol": "voyage.vessel.v1", "Authorization": "Bearer " + credential["token"],
                "X-Voyage-Grant": credential["grant_id"],
                **({"X-Voyage-Vessel": credential["vessel_id"]} if "vessel_id" in credential else {})}
            self.socket.sendall(("GET /v1/vessel/socket HTTP/1.1\r\n" +
                "".join(f"{name}: {value}\r\n" for name, value in headers.items()) + "\r\n").encode())
            response = b""
            while not response.endswith(b"\r\n\r\n"):
                response += self.exact(1)
                assert len(response) < 16384, "oversized upgrade"
            assert response.startswith(b"HTTP/1.1 101"), "scoped WebSocket upgrade refused"
            expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
            assert b"sec-websocket-accept: " + expected.lower() in response.lower()
            assert self.receive()["type"] == "hello"
        except BaseException:
            self.socket.close()
            raise

    def exact(self, size):
        data = bytearray()
        while len(data) < size:
            chunk = self.socket.recv(size - len(data))
            if not chunk:
                raise EOFError("socket closed")
            data.extend(chunk)
        return bytes(data)

    def send(self, value=None, opcode=1, payload=None):
        payload = json.dumps(value).encode() if payload is None else payload
        mask = os.urandom(4)
        length = len(payload)
        header = bytes([0x80 | opcode, 0x80 | length]) if length < 126 else bytes([0x80 | opcode, 0xfe]) + struct.pack("!H", length)
        with self.lock:
            self.socket.sendall(header + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))

    def receive(self):
        while True:
            first, second = self.exact(2)
            assert first & 0x80 and not second & 0x80, "unexpected fragmented/masked server frame"
            length = second & 127
            if length == 126:
                length = struct.unpack("!H", self.exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self.exact(8))[0]
            assert length <= 8 * 1024 * 1024, "oversized public frame"
            payload = self.exact(length)
            opcode = first & 15
            if opcode == 8:
                raise EOFError("peer closed WebSocket")
            if opcode == 9:
                self.send(opcode=10, payload=payload)
                continue
            if opcode == 10:
                continue
            assert opcode == 1
            return json.loads(payload)

    def subscribe(self, session, incarnation, after):
        request_id = str(uuid.uuid4())
        self.send({"type": "subscribe", "request_id": request_id, "request": {"protocol": 1,
            "subscriptions": [{"session_id": session, "incarnation": incarnation, "after": after, "projection": "public-v2"}]}})
        reply = self.receive()
        assert reply == {"type": "subscribed", "request_id": request_id}, reply
        return request_id

    def close(self):
        try:
            self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.socket.close()


def tcp_state(server_port, client_port):
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if int(fields[1].split(":")[1], 16) == server_port and int(fields[2].split(":")[1], 16) == client_port:
            return {"state": fields[3], "queued": int(fields[4].split(":")[0], 16)}
    return {"state": "absent", "queued": 0}


def rss(pid):
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) * 1024
    raise AssertionError("missing gateway RSS")


def run(fixture, gateway):
    session = fixture.session()
    grant = gateway.grant(session, ["observe", "history"], lifetime=600000)
    command = fixture.submit(session, "Synthetic socket backpressure, exactly one inference")
    fixture.command(session, command)
    assert fixture.provider.entered.wait(10)
    snapshot = fixture.command(session, {"op": "snapshot"})
    incarnation = fixture.request({"op": "inspect", "session_id": session})["incarnation"]
    initial_cursor = snapshot["observation_cursor"]
    sockets = []
    stopped = threading.Event()
    state = {"text": "", "cursor": initial_cursor, "pages": 0, "max_page_bytes": 0, "errors": [], "replies": {}}
    condition = threading.Condition()
    pings = []
    threads = []
    try:
        fast = WebSocket(gateway.origin, grant); sockets.append(fast)
        fast.subscribe(session, incarnation, initial_cursor)
        slow = WebSocket(gateway.origin, grant, slow=True); sockets.append(slow)
        for _ in range(16):
            slow.subscribe(session, incarnation, initial_cursor)

        def read_fast():
            try:
                while not stopped.is_set():
                    frame = fast.receive()
                    with condition:
                        if frame["type"] == "reply":
                            state["replies"][frame["request_id"]] = frame["response"]
                        elif frame["type"] == "event":
                            event = frame["event"]
                            assert event.get("error") is None and event["incarnation"] == incarnation, event
                            page = event["result"]
                            assert page["projection"] == "public-v2" and not page["replay_gap"], page
                            assert len(page["events"]) <= 128
                            for item in page["events"]:
                                assert item["cursor"] > state["cursor"]
                                state["cursor"] = item["cursor"]
                                assert len(json.dumps(item["payload"], ensure_ascii=False).encode()) <= 32768
                                if item["kind"] == "text_delta":
                                    assert item["payload"]["offset"] == len(state["text"].encode())
                                    state["text"] += item["payload"]["text"]
                            assert page["cursor"] == state["cursor"]
                            state["pages"] += 1
                            state["max_page_bytes"] = max(state["max_page_bytes"], len(json.dumps(frame).encode()))
                        condition.notify_all()
            except Exception as error:
                if not stopped.is_set():
                    with condition:
                        state["errors"].append(str(error)); condition.notify_all()

        def keep_slow_alive():
            while not stopped.wait(1):
                try:
                    slow.send(opcode=9, payload=b"fixture-liveness")
                    pings.append(time.monotonic())
                except OSError:
                    return

        threads = [threading.Thread(target=read_fast), threading.Thread(target=keep_slow_alive)]
        for thread in threads:
            thread.start()
        started = time.monotonic()
        samples = []
        latencies = []
        fixture.provider.begin.set()
        while time.monotonic() - started < 90:
            elapsed = time.monotonic() - started
            sample = tcp_state(urlsplit(gateway.origin).port, slow.port)
            samples.append({"seconds": elapsed, **sample, "rss": rss(gateway.process.pid), "fast_bytes": len(state["text"]), "pings": len(pings)})
            assert not state["errors"], state["errors"]
            request_id = str(uuid.uuid4())
            sent = time.monotonic()
            fast.send({"type": "command", "request_id": request_id, "request": {"protocol": 1, "command": {"op": "snapshot", "session_id": session}}})
            with condition:
                assert condition.wait_for(lambda: request_id in state["replies"] or state["errors"], timeout=8), "fast peer command stalled"
                assert not state["errors"], state["errors"]
                reply = state["replies"].pop(request_id)
            assert reply.get("error") is None, reply
            latencies.append(time.monotonic() - sent)
            if len(state["text"]) == len(TEXT) * CHUNKS:
                break
            time.sleep(0.2)
        assert state["text"] == TEXT * CHUNKS, "fast peer failed to receive exact streaming text"
        congested = [sample for sample in samples if sample["queued"] > slow.receive_buffer * 4]
        assert congested, "test never reached real kernel socket backpressure"
        closed = [sample for sample in samples if sample["state"] != "01" and sample["seconds"] > congested[0]["seconds"]]
        assert closed, "stalled connection did not leave ESTABLISHED within the bounded run"
        first_closed = closed[0]
        assert first_closed["seconds"] - congested[0]["seconds"] < 30, "backpressure release was not bounded"
        recent_pings = [instant for instant in pings if instant - started <= first_closed["seconds"]]
        assert recent_pings and first_closed["seconds"] - (recent_pings[-1] - started) < 3, "heartbeat expiry could explain closure"
        assert samples[-1]["fast_bytes"] > first_closed["fast_bytes"], "producer/fast peer did not continue after slow peer detached"
        assert max(sample["rss"] for sample in samples) - samples[0]["rss"] < 128 * 1024 * 1024, "gateway retained memory exceeded fixture budget"

        recovered = WebSocket(gateway.origin, grant); sockets.append(recovered)
        recovered.subscribe(session, incarnation, initial_cursor)
        gap = recovered.receive()["event"]["result"]
        assert gap["replay_gap"] and gap["recovery"] == "snapshot", gap
        current = gateway.call(grant, session, {"op": "snapshot"})
        assert current["error"] is None
        current = current["result"]
        assert current["run"]["partial_text_bytes"] == len(TEXT) * CHUNKS
        assert current["run"]["partial_text"] == (TEXT * CHUNKS)[:65536]
        assert current["run"]["partial_text_truncated"]
        assert current["observation_cursor"] >= state["cursor"]
        assert len(fixture.provider.bodies) == 1 and not fixture.provider.errors
        fixture.provider.finish.set()
        completed = fixture.finished(session)
        assert completed["run"]["state"] == "completed"
        fixture.record("public-v2-native-tcp-backpressure", {
            "binary_sha256": {name: hashlib.sha256((fixture.binaries / name).read_bytes()).hexdigest() for name in ("vessel", "voyage")},
            "session": session, "initial_cursor": initial_cursor, "fast_cursor": state["cursor"],
            "canonical_cursor": completed["observation_cursor"], "revision": completed["revision"],
            "stream_bytes": len(state["text"]), "delta_count": CHUNKS, "slow_subscriptions": 16,
            "slow_receive_buffer": slow.receive_buffer, "first_congested": congested[0], "first_closed": first_closed,
            "fast_pages": state["pages"], "max_page_bytes": state["max_page_bytes"],
            "max_command_seconds": max(latencies), "samples": samples, "inferences": 1,
            "recovery": "explicit gap and authorized canonical snapshot", "errors": state["errors"]})
    finally:
        stopped.set()
        fixture.provider.abort.set(); fixture.provider.begin.set(); fixture.provider.finish.set()
        for peer in sockets:
            peer.close()
        for thread in threads:
            thread.join(timeout=10)
            assert not thread.is_alive(), "fixture reader did not stop"


def main():
    assert __debug__, "assertions are required"
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    fixture.provider.RequestHandlerClass = Provider
    fixture.provider.errors = []
    for name in ("entered", "begin", "sent", "finish", "abort"):
        setattr(fixture.provider, name, threading.Event())
    gateway = None
    try:
        fixture.start()
        gateway = Gateway(fixture)
        run(fixture, gateway)
    finally:
        fixture.provider.abort.set(); fixture.provider.begin.set(); fixture.provider.finish.set()
        if gateway:
            gateway.close()
        fixture.close()


if __name__ == "__main__":
    main()

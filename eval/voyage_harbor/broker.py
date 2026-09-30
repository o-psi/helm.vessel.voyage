"""Host-only bounded credential broker. No real OAuth tokens enter task containers."""

import argparse
import asyncio
import fcntl
import json
import os
from pathlib import Path
import secrets
import signal
import stat
import time
import uuid
from aiohttp import web
import httpx


class Broker:
    def __init__(
        self, directory, registry, account_id, token, mode="live", max_requests=6000
    ):
        self.directory, self.registry, self.account_id = directory, registry, account_id
        self.token, self.mode, self.maximum = token, mode, max_requests
        self.requests = 0
        self.identity = None
        self.stats = {
            "mode": mode,
            "responses": 0,
            "models": 0,
            "upstream_statuses": {},
            "usage": [],
        }
        self.client = httpx.AsyncClient(
            timeout=httpx.Timeout(1800, connect=30), follow_redirects=False
        )

    def credentials(self):
        root = self.registry.parent
        descriptor = os.open(root / "actor.lock", os.O_RDONLY | os.O_NOFOLLOW)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_SH | fcntl.LOCK_NB)
            fd = os.open(self.registry, os.O_RDONLY | os.O_NOFOLLOW)
            try:
                metadata = os.fstat(fd)
                if (
                    not stat.S_ISREG(metadata.st_mode)
                    or metadata.st_uid != os.getuid()
                    or metadata.st_mode & 0o077
                ):
                    raise ValueError("Unsafe account registry")
                raw = os.read(fd, 65537)
                if len(raw) > 65536:
                    raise ValueError("Oversized registry")
                db = json.loads(raw)
            finally:
                os.close(fd)
        finally:
            os.close(descriptor)
        record = next(
            a for a in db["accounts"] if a["descriptor"]["id"] == self.account_id
        )
        if record["descriptor"]["state"] != "ready" or record.get("refresh"):
            raise ValueError("Account unavailable")
        connection = next(
            c
            for c in db["connections"]
            if c["id"] == record["descriptor"]["connection_id"]
        )
        if (
            connection["endpoint"].rstrip("/")
            != "https://chatgpt.com/backend-api/codex"
        ):
            raise ValueError("Unapproved subscription endpoint")
        identity = (
            record["descriptor"]["identity_generation"],
            record["descriptor"]["capability_revision"],
            connection["revision"],
        )
        if self.identity is not None and self.identity != identity:
            raise ValueError(
                "Account authority changed; explicit reconciliation required"
            )
        self.identity = identity
        tokens = record["credential"]["OAuth"]
        if tokens["expires_at"] <= time.time() + 300:
            raise ValueError(
                "Refresh the named account through native Voyage before continuing"
            )
        return tokens

    async def handle(self, request):
        auth = request.headers.get("Authorization", "")
        if not secrets.compare_digest(auth, "Bearer " + self.token):
            return web.json_response(
                {"error": {"message": "Benchmark authorization denied"}}, status=403
            )
        session = request.headers.get("X-Voyage-Benchmark-Session")
        try:
            uuid.UUID(session or "")
        except ValueError:
            return web.json_response(
                {"error": {"message": "Missing trial identity"}}, status=403
            )
        route = request.path
        if (request.method, route) not in [("GET", "/models"), ("POST", "/responses")]:
            return web.json_response(
                {"error": {"message": "Benchmark route denied"}}, status=403
            )
        body = await request.read()
        if route == "/responses":
            try:
                value = json.loads(body)
                if value.get("model") != "gpt-6.1-sol" or value.get(
                    "reasoning", {}
                ).get("effort") not in ("medium", "high", "xhigh"):
                    raise ValueError()
            except (ValueError, TypeError):
                return web.json_response(
                    {"error": {"message": "Benchmark model/settings denied"}},
                    status=403,
                )
            if self.requests >= self.maximum:
                return web.json_response(
                    {"error": {"message": "Benchmark request allowance exhausted"}},
                    status=429,
                )
            self.requests += 1
            self.stats["responses"] += 1
        else:
            self.stats["models"] += 1
        if self.mode != "live":
            return await self.mock(request, body)
        try:
            tokens = self.credentials()
        except (OSError, ValueError, KeyError, StopIteration):
            return web.json_response(
                {
                    "error": {
                        "message": "Selected subscription account unavailable; native reconciliation required"
                    }
                },
                status=503,
            )
        headers = {
            "Authorization": "Bearer " + tokens["access_token"],
            "ChatGPT-Account-Id": tokens["account_id"],
            "Content-Type": "application/json",
            "originator": "helm",
            "User-Agent": request.headers.get("User-Agent", "helm/benchmark"),
        }
        if request.headers.get("OpenAI-Beta"):
            headers["OpenAI-Beta"] = request.headers["OpenAI-Beta"]
        params = {"client_version": "99.99.99"} if route == "/models" else None
        upstream = self.client.build_request(
            request.method,
            "https://chatgpt.com/backend-api/codex" + route,
            headers=headers,
            content=body,
            params=params,
        )
        try:
            response = await self.client.send(upstream, stream=True)
        except httpx.HTTPError:
            return web.json_response(
                {"error": {"message": "Subscription transport unavailable"}}, status=502
            )
        code = str(response.status_code)
        self.stats["upstream_statuses"][code] = (
            self.stats["upstream_statuses"].get(code, 0) + 1
        )
        downstream = web.StreamResponse(
            status=response.status_code,
            headers={
                "Content-Type": response.headers.get("Content-Type", "application/json")
            },
        )
        pending = b""
        try:
            await downstream.prepare(request)
            async for block in response.aiter_bytes():
                await downstream.write(block)
                if route == "/responses":
                    pending += block
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n", 1)
                        if line.startswith(b"data: "):
                            try:
                                event = json.loads(line[6:])
                                if event.get("type") == "response.completed":
                                    self.observe_usage(
                                        event.get("response", {}), session, value
                                    )
                            except (ValueError, TypeError):
                                pass
                    if len(pending) > 4 * 1024 * 1024:
                        pending = b""
            await downstream.write_eof()
        except (ConnectionError, asyncio.CancelledError):
            pass
        finally:
            await response.aclose()
            self.save_stats()
        return downstream

    async def mock(self, request, body):
        if request.path == "/models":
            return web.json_response(
                {
                    "models": [
                        {
                            "slug": "gpt-6.1-sol",
                            "display_name": "GPT-6.1 Sol",
                            "supported_reasoning_levels": [
                                {"effort": x, "description": x}
                                for x in ("medium", "high", "xhigh")
                            ],
                        }
                    ]
                }
            )
        value = json.loads(body)
        results = any(
            x.get("type") == "function_call_output" for x in value.get("input", [])
        ) or not value.get("tools")
        if self.mode == "stall" and results:
            await asyncio.sleep(300)
        output = (
            [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Wrote the requested file."}
                    ],
                }
            ]
            if results
            else [
                {
                    "type": "function_call",
                    "id": "fc_smoke",
                    "call_id": "call_smoke",
                    "name": "shell",
                    "arguments": json.dumps(
                        {"command": "printf 'voyage-harbor-smoke\\n' > /app/answer.txt"}
                    ),
                }
            ]
        )
        result = {
            "id": "resp_smoke_" + str(self.requests),
            "object": "response",
            "status": "completed",
            "output": output,
            "usage": {"input_tokens": 100, "output_tokens": 20},
        }
        if not value.get("stream", False):
            self.save_stats()
            return web.json_response(result)
        payload = (
            "data: "
            + json.dumps({"type": "response.completed", "response": result})
            + "\n\n"
        )
        self.observe_usage(result, request.headers["X-Voyage-Benchmark-Session"], value)
        self.save_stats()
        return web.Response(text=payload, content_type="text/event-stream")

    def observe_usage(self, response, session, request):
        usage = response.get("usage") or {}
        self.stats["usage"].append(
            {
                "session_id": session,
                "model_requested": request.get("model"),
                "effort_requested": request.get("reasoning", {}).get("effort"),
                "model_reported": response.get("model"),
                "service_tier_reported": response.get("service_tier"),
                "input_tokens": usage.get("input_tokens"),
                "output_tokens": usage.get("output_tokens"),
                "cached_input_tokens": (usage.get("input_tokens_details") or {}).get(
                    "cached_tokens"
                ),
                "reasoning_output_tokens": (
                    usage.get("output_tokens_details") or {}
                ).get("reasoning_tokens"),
            }
        )

    def save_stats(self):
        target = self.directory / "broker-stats.json"
        tmp = target.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.stats, indent=2))
        tmp.chmod(0o600)
        tmp.replace(target)


async def serve(args):
    directory = args.directory.resolve()
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(directory, 0o700)
    socket_dir = directory / "socket"
    socket_dir.mkdir(mode=0o711, exist_ok=True)
    token = secrets.token_urlsafe(32)
    token_path = directory / "proxy-token"
    token_path.write_text(token)
    token_path.chmod(0o600)
    broker = Broker(
        directory, args.registry, args.account, token, args.mode, args.max_requests
    )
    app = web.Application(client_max_size=16 * 1024 * 1024)
    app.router.add_route("*", "/{path:.*}", broker.handle)
    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    path = socket_dir / "provider.sock"
    if path.exists():
        raise RuntimeError("Refusing to replace an existing broker socket")
    await web.UnixSite(runner, str(path)).start()
    os.chmod(path, 0o666)
    stop = asyncio.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        asyncio.get_running_loop().add_signal_handler(sig, stop.set)
    pid_path = directory / "broker.pid"
    pid_path.write_text(str(os.getpid()))
    pid_path.chmod(0o600)
    print(
        json.dumps({"ready": True, "mode": args.mode, "socket": str(path)}), flush=True
    )
    try:
        await stop.wait()
    finally:
        broker.save_stats()
        await runner.cleanup()
        await broker.client.aclose()
        path.unlink(missing_ok=True)
        token_path.unlink(missing_ok=True)
        pid_path.unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument(
        "--registry",
        type=Path,
        default=Path.home() / ".local/share/helm/accounts/registry.json",
    )
    parser.add_argument("--account", required=True)
    parser.add_argument("--mode", choices=["live", "mock", "stall"], default="mock")
    parser.add_argument("--max-requests", type=int, default=6000)
    asyncio.run(serve(parser.parse_args()))

"""Container-local HTTP relay to a host Unix socket; holds no upstream credentials."""

import argparse
import http.client
import http.server
import socket
import ssl


class UnixConnection(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__("localhost", timeout=1800)
        self.path = path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.path)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def forward(self):
        path = self.path.removeprefix("/backend-api/codex")
        if self.command not in ("GET", "POST") or path.split("?")[0] not in (
            "/models",
            "/responses",
        ):
            self.send_error(403)
            return
        size = int(self.headers.get("Content-Length", "0"))
        if size < 0 or size > 16 * 1024 * 1024:
            self.send_error(413)
            return
        conn = UnixConnection(self.server.socket_path)
        try:
            body = self.rfile.read(size)
            headers = {
                k: v
                for k, v in self.headers.items()
                if k.lower()
                in (
                    "authorization",
                    "content-type",
                    "originator",
                    "user-agent",
                    "openai-beta",
                )
            }
            headers["X-Voyage-Benchmark-Session"] = self.server.session_id
            conn.request(self.command, path, body=body, headers=headers)
            response = conn.getresponse()
            self.send_response(response.status)
            self.send_header(
                "Content-Type", response.getheader("Content-Type", "application/json")
            )
            self.send_header("Connection", "close")
            self.end_headers()
            while block := response.read1(65536):
                self.wfile.write(block)
                self.wfile.flush()
        except (OSError, http.client.HTTPException):
            self.close_connection = True
        finally:
            conn.close()
            self.close_connection = True

    do_GET = forward
    do_POST = forward


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    parser.add_argument("--port", type=int, default=443)
    parser.add_argument("--session", required=True)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(
        "/run/voyage-benchmark/provider.crt", "/run/voyage-benchmark/provider.key"
    )
    server.socket = context.wrap_socket(server.socket, server_side=True)
    server.socket_path = args.socket
    server.session_id = args.session
    server.serve_forever()

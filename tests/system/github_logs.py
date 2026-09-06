#!/usr/bin/env python3
"""Actual credential-free TLS receiver using only a test-binary transport seam."""
import argparse
import http.server
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-binary", required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="helm-github-logs-") as directory:
        root = Path(directory)
        certificate, key = root / "certificate.pem", root / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-keyout", str(key), "-out", str(certificate), "-days", "1",
                        "-subj", "/CN=logs.fixture.test", "-addext", "subjectAltName=DNS:logs.fixture.test"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20)
        requests = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *unused):
                pass
            def do_GET(self):
                requests.append((self.path, {key.lower(): value for key, value in self.headers.items()}))
                mode = self.path.split("?", 1)[0][1:]
                (root / f"{mode}.ready").touch()
                if mode == "redirect":
                    self.send_response(302)
                    self.send_header("Location", "/must-not-follow")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if mode == "expired":
                    self.send_response(403)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if mode == "cancel":
                    time.sleep(0.5)
                body = (b"x" * (1024 * 1024 + (1 if mode == "bounded" else 0))
                        if mode in ("bounded", "exact") else b"workflow log: succeeded\n")
                self.send_response(200)
                self.send_header("Content-Length", str(len(body) + (100 if mode == "partial" else 0)))
                self.send_header("Set-Cookie", "must-not-be-retained=secret")
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
                    pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for mode in ("success", "bounded", "exact", "redirect", "expired", "partial", "cancel"):
                before = len(requests)
                environment = os.environ.copy()
                environment.update(HELM_GITHUB_LOG_DRIVER=mode, HELM_GITHUB_LOG_PORT=str(server.server_port),
                                   HELM_GITHUB_LOG_CERT=str(certificate), HELM_GITHUB_LOG_READY=str(root / f"{mode}.ready"))
                result = subprocess.run([args.test_binary, "--exact", "github::logs::tls_fixture::tls_driver", "--nocapture"],
                                        env=environment, text=True, capture_output=True, timeout=10)
                assert result.returncode == 0 and "TLS_DRIVER_PASS" in result.stdout, (mode, result.stdout, result.stderr)
                assert len(requests) == before + 1, (mode, len(requests) - before)
                path, headers = requests[-1]
                assert path == f"/{mode}?signed=fixture-private-query"
                assert headers["host"] == f"logs.fixture.test:{server.server_port}"
                assert headers.get("accept") == "text/plain"
                for forbidden in ("authorization", "cookie", "referer", "x-github-api-version", "proxy-authorization"):
                    assert forbidden not in headers, (mode, forbidden)
                assert "fixture-authenticated-api-token" not in str(headers)
                print(f"PASS signed-log TLS {mode}")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("PASS 7 actual secondary HTTPS transport cases; public DNS rejection is separately unit-tested")


if __name__ == "__main__":
    main()

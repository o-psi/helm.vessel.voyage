#!/usr/bin/env python3
"""Actual credential-free TLS receiver using only a test-binary transport seam."""
import argparse
import http.server
import json
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
        ca, ca_key, request = root / "ca.pem", root / "ca-key.pem", root / "server.csr"
        extensions = root / "server.ext"
        extensions.write_text("basicConstraints=critical,CA:FALSE\n"
                              "keyUsage=critical,digitalSignature,keyEncipherment\n"
                              "extendedKeyUsage=serverAuth\n"
                              "subjectAltName=DNS:logs.fixture.test\n")
        commands = [
            ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
             "-keyout", str(ca_key), "-out", str(ca), "-days", "1",
             "-subj", "/CN=GitHub log fixture CA", "-addext", "basicConstraints=critical,CA:TRUE",
             "-addext", "keyUsage=critical,keyCertSign,cRLSign"],
            ["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
             "-keyout", str(key), "-out", str(request), "-subj", "/CN=logs.fixture.test"],
            ["openssl", "x509", "-req", "-in", str(request), "-CA", str(ca),
             "-CAkey", str(ca_key), "-CAcreateserial", "-out", str(certificate),
             "-days", "1", "-extfile", str(extensions)],
        ]
        for command in commands:
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=20)
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
        api_requests = []
        class API(http.server.BaseHTTPRequestHandler):
            def log_message(self, *unused):
                pass
            def do_GET(self):
                api_requests.append((self.path, {key.lower():value for key,value in self.headers.items()}))
                self.send_response(302 if self.path.endswith("/logs") else 200)
                if self.path.endswith("/logs"):
                    self.send_header("Location", "https://logs.fixture.test/pipeline?signed=fixture-private-query")
                    body = b""
                else:
                    if self.path.endswith("/pulls/1"):
                        value = {"number":1,"html_url":"https://github.com/fixture/repository/pull/1",
                                 "head":{"sha":"a"*40},"base":{"sha":"b"*40,"repo":{"id":42},"ref":"main"}}
                    elif self.path.endswith("/jobs/7"):
                        value = {"id":7,"head_sha":"a"*40,"run_id":11,"run_attempt":2,"status":"completed","conclusion":"success",
                                 "check_run_url":"https://not-followed.invalid/check-runs/999"}
                    elif self.path.endswith("/runs/11"):
                        value = {"id":11,"head_sha":"a"*40}
                    else:
                        value = {"unexpected":self.path}
                    body = json.dumps(value).encode()
                    self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        api = http.server.ThreadingHTTPServer(("127.0.0.1", 0), API)
        api_thread = threading.Thread(target=api.serve_forever, daemon=True)
        api_thread.start()
        try:
            for mode in ("success", "bounded", "exact", "redirect", "expired", "partial", "cancel", "pipeline"):
                before = len(requests)
                environment = os.environ.copy()
                environment.update(HELM_GITHUB_LOG_DRIVER=mode, HELM_GITHUB_LOG_PORT=str(server.server_port),
                                   HELM_GITHUB_LOG_API_PORT=str(api.server_port), HELM_GITHUB_LOG_CERT=str(ca), HELM_GITHUB_LOG_READY=str(root / f"{mode}.ready"))
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
                if mode == "pipeline":
                    prefix = "/repos/fixture/repository/"
                    assert [path for path, _ in api_requests] == [prefix+suffix for suffix in
                        ("pulls/1", "actions/jobs/7", "actions/runs/11", "pulls/1", "actions/jobs/7/logs", "pulls/1")]
                    for _, api_headers in api_requests:
                        assert api_headers["authorization"] == "Bearer fixture-authenticated-api-token"
                        assert api_headers["x-github-api-version"] == "2026-03-10"
                else:
                    assert not api_requests
                print(f"PASS signed-log TLS {mode}")
        finally:
            api.shutdown()
            api.server_close()
            api_thread.join(timeout=2)
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("PASS 8 actual secondary HTTPS transport cases including authenticated API pipeline; public DNS rejection is separately unit-tested")


if __name__ == "__main__":
    main()

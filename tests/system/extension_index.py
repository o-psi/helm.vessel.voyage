#!/usr/bin/env python3
"""Offline real TLS index acquisition, digest/origin/redirect/size rejection."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
from extensions import archive


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    with tempfile.TemporaryDirectory(prefix="helm-extension-index-") as temporary:
        root = Path(temporary)
        key, cert = root / "key.pem", root / "cert.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key),
                        "-out", str(cert), "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost", "-addext", "basicConstraints=critical,CA:FALSE"],
                       check=True, capture_output=True, timeout=20)
        bodies, status, requests = {}, {}, []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                requests.append((self.path, dict(self.headers)))
                data = bodies.get(self.path, b"missing")
                code = status.get(self.path, 200)
                self.send_response(code)
                if code == 302:
                    self.send_header("Location", "/archive")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cert, key)
        server.socket = tls.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        origin = f"https://localhost:{server.server_port}"
        config = root / "index.json"
        env = dict(os.environ, HOME=str(root / "home"), XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"))
        package = json.dumps(archive("INDEX_CANARY")).encode()
        bodies["/archive"] = package
        def configure(url=None, sha=None, ca=True):
            index = {"format": 1, "packages": [{"id": "example", "url": url or origin + "/archive",
                                                "sha256": sha or hashlib.sha256(package).hexdigest()}]}
            bodies["/index"] = json.dumps(index).encode()
            record = {"format": 1, "url": origin + "/index", "sha256": hashlib.sha256(bodies["/index"]).hexdigest()}
            if ca:
                record["ca_certificate"] = str(cert)
            config.write_text(json.dumps(record))
        def run(success=True):
            result = subprocess.run([str(helm), "--workspace", str(root), "extension", "fetch", str(config), "example"],
                                    env=env, capture_output=True, text=True, timeout=30)
            assert (result.returncode == 0) == success, (result.returncode, result.stdout, result.stderr)
            assert "INDEX_CANARY" not in result.stdout + result.stderr
            return result
        try:
            configure(ca=False)
            run(False)  # Real self-signed TLS refusal.
            configure(url="https://other.invalid/archive")
            run(False)
            assert not any(path == "/archive" for path, _ in requests)
            configure(url="http://localhost/archive")
            run(False)
            configure()
            status["/index"] = 302
            run(False)
            status.clear()
            configure()
            status["/archive"] = 302
            run(False)
            status.clear()
            configure()
            record = json.loads(config.read_text())
            record["sha256"] = "0" * 64
            config.write_text(json.dumps(record))
            run(False)
            configure(sha="0" * 64)
            run(False)
            configure()
            bodies["/archive"] = b"x" * 65537
            run(False)
            bodies["/archive"] = package
            configure()
            bodies["/index"] = b"x" * 65537
            run(False)
            configure()
            run()
            for _, headers in requests:
                assert "Authorization" not in headers and "Cookie" not in headers
            installed = subprocess.run([str(helm), "--workspace", str(root), "extension", "inspect", "example"],
                                       env=env, capture_output=True, text=True, timeout=10, check=True)
            assert not json.loads(installed.stdout)["active"]
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("extension HTTPS index acquisition and refusals: PASS")


if __name__ == "__main__":
    main()

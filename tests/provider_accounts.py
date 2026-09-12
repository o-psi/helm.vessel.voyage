#!/usr/bin/env python3
"""Offline Linux named-account process regression. No operator credentials/provider IO."""
import argparse
import io
import json
from pathlib import Path
import subprocess
import time
import unittest
import uuid
import urllib.request

from voyage_fixture import Fixture, ProviderHandler, wait_for


class AccountingHandler(ProviderHandler):
    def do_GET(self):
        assert self.path == "/v1/models"
        payload = json.dumps({"data": [{"id": "fixture-model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        raw = self.rfile.read(int(self.headers["Content-Length"]))
        body = json.loads(raw)
        prompt = next(m["content"] for m in reversed(body["messages"]) if m["role"] == "user")
        with self.server.guard:
            self.server.account_requests.append((prompt, self.headers.get("Authorization"), body))
        if prompt == "rotate-during-run" and self.server.count(prompt) == 1:
            with self.server.guard:
                self.server.requests[prompt] += 1
            # Echo a synthetic rotated secret split across stream frames. The
            # admitted agent must learn the dispatch credential before publishing.
            chunks = [{"choices": [{"index": 0, "delta": {"content": part}, "finish_reason": None}]}
                      for part in ("synthetic-personal-", "rotated")]
            chunks.append({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
            payload = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks) + "data: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        # Reuse the existing bounded gate/stream fixture, not another runtime suite.
        self.rfile = io.BytesIO(raw)
        super().do_POST()


class AccountsFixture(Fixture):
    def __init__(self, binary):
        super().__init__(binary)
        self.env.update(ACCOUNT_PERSONAL="synthetic-personal", ACCOUNT_WORK="synthetic-work",
                        ACCOUNT_PERSONAL_ROTATED="synthetic-personal-rotated")
        self.provider.RequestHandlerClass = AccountingHandler
        self.provider.account_requests = []
        host_config = Path(self.env["XDG_CONFIG_HOME"]) / "helm/config.toml"
        host_config.parent.mkdir(mode=0o700)
        host_config.write_bytes(self.config.read_bytes())
        host_config.chmod(0o600)

    def request(self, command, *, envelope=False):
        credential = json.loads((self.directory / "process-http.json").read_text())
        payload = json.dumps({"protocol": 1, "command": command}).encode()
        request = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=payload, headers={"Authorization": "Bearer " + credential["token"],
                                  "Content-Type": "application/json"}, method="POST")
        with urllib.request.urlopen(request, timeout=30) as incoming:
            response = json.loads(incoming.read(4 * 1024 * 1024 + 1))
        assert response["protocol"] == 1
        if envelope:
            return response
        assert response.get("error") is None, response
        return response["result"]

    def cli(self, *args, ok=True):
        result = subprocess.run([str(self.bin_dir / "vessel"), "auth", "accounts", *args],
                                env=self.env, cwd=self.workspace, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=15)
        assert (result.returncode == 0) == ok, "synthetic account CLI returned unexpected status"
        if not ok or args[0] == "logout":
            return None
        return json.loads(result.stdout)

    def enroll_api(self, alias, environment, connection):
        descriptor = self.cli("add", "--connection", connection["id"], "--account", alias,
                              "--env", environment)
        return {"account_id": descriptor["id"], "connection_id": connection["id"],
                "identity_generation": descriptor["identity_generation"],
                "connection_revision": connection["revision"], "transport": "openai_chat"}

    def new_account(self, account):
        session = str(uuid.uuid4())
        self.sessions[session] = None  # Track before effects, including uncertain start.
        envelope = {"op": "start_account", "command_id": str(uuid.uuid4()),
                    "session_id": session, "workspace": str(self.workspace), "account": account,
                    "model": "fixture-model", "reasoning_effort": None, "service_tier": None}
        process = self.request(envelope)
        self.sessions[session] = process["incarnation"]
        return session, envelope

    def raw_command(self, session, command):
        envelope = self.request({"session_id": session, **command}, envelope=True)
        if envelope.get("error") is not None:
            return {"error": envelope["error"], "result": None}
        reply = envelope["result"]
        self.sessions[session] = reply["incarnation"]
        return {"error": None, "result": reply["result"]}

    def reached_provider(self, session, prompt):
        wait_for(lambda: self.provider.count(prompt) == 1, "named-account provider dispatch")
        assert self.snapshot(session)["run"]["state"] == "running"

    def authorization(self, prompt):
        with self.provider.guard:
            return [auth for name, auth, _ in self.provider.account_requests if name == prompt]


class NamedAccounts(unittest.TestCase):
    def setUp(self):
        self.f = AccountsFixture(BIN_DIR)
        self.addCleanup(self.f.close)
        print(f"\nSynthetic account evidence: {self.f.root}", flush=True)
        connection = self.f.cli("connect", "--label", "Synthetic OpenAI API", "--endpoint",
            f"http://127.0.0.1:{self.f.provider.server_port}/v1", "--transports", "openai-chat,openai-responses")
        self.personal = self.f.enroll_api("personal", "ACCOUNT_PERSONAL", connection)
        self.work = self.f.enroll_api("work", "ACCOUNT_WORK", connection)
        self.f.cli("add", "--connection", connection["id"], "--account", "personal",
                   "--env", "ACCOUNT_WORK", ok=False)
        self.f.start()
        self.f.request({"op": "account_set_default", "command_id": str(uuid.uuid4()),
                        "workspace": str(self.f.workspace), "account": self.personal,
                        "expected_revision": 0})

    def switch(self, session, account):
        command = self.f.mutation(session, "set_account_inference", account=account,
            model="fixture-model", reasoning_effort=None, service_tier=None)
        receipt = self.f.command(session, command)
        return command, receipt

    def test_two_accounts_live_switch_resume_branch_and_exact_receipts(self):
        f = self.f
        a, start = f.new_account(self.personal)
        b, _ = f.new_account(self.work)
        first = f.submit(a, "personal-held")
        second = f.submit(b, "work-held")
        f.reached_provider(a, "personal-held")
        f.reached_provider(b, "work-held")
        self.assertEqual(f.authorization("personal-held"), ["Bearer synthetic-personal"])
        self.assertEqual(f.authorization("work-held"), ["Bearer synthetic-work"])
        command, receipt = self.switch(a, self.work)
        snapshot = f.snapshot(a)
        self.assertEqual(snapshot["inference_current"]["account"], self.personal)
        self.assertEqual(snapshot["inference"]["account"], self.work)
        self.assertEqual(f.command(a, command), receipt)
        conflict = {**command, "account": self.personal}
        self.assertIsNotNone(f.raw_command(a, conflict)["error"])
        first[0].set()
        second[0].set()
        f.finished(a)
        f.finished(b)
        wait_for(lambda: f.request({"op": "inspect", "session_id": a})["state"] == "suspended",
                 "suspended named-account owner")
        resumed = f.submit(a, "work-after-resume")
        f.reached_provider(a, "work-after-resume")
        self.assertEqual(f.authorization("work-after-resume"), ["Bearer synthetic-work"])
        resumed[0].set()
        f.finished(a)
        snap = f.snapshot(a)
        branch = str(uuid.uuid4())
        f.sessions[branch] = None
        info = f.request({"op": "inspect", "session_id": a})
        process = f.request({"op": "branch", "command_id": str(uuid.uuid4()),
            "session_id": a, "incarnation": info["incarnation"], "expected_revision": snap["revision"],
            "expires_at_ms": int(time.time() * 1000) + 120_000, "branch_id": branch, "name": "Account branch"})
        f.sessions[branch] = process["incarnation"]
        self.assertEqual(f.snapshot(branch)["inference"]["account"], self.work)
        third = f.submit(branch, "branch-work")
        f.reached_provider(branch, "branch-work")
        self.assertEqual(f.authorization("branch-work"), ["Bearer synthetic-work"])
        third[0].set()
        f.finished(branch)
        self.assertEqual(f.request({**start, "op": "resolve_start_account"})["session_id"], a)
        self.assertEqual(f.provider.errors, [])
        public = json.dumps([f.snapshot(a), f.snapshot(b), f.snapshot(branch)])
        for secret in ("synthetic-personal", "synthetic-work", "ACCOUNT_PERSONAL", "ACCOUNT_WORK"):
            self.assertNotIn(secret, public)

    def test_scoped_catalogue_and_selection_use_current_explicit_authority(self):
        f = self.f
        session, _ = f.new_account(self.personal)
        host = f.request({"op": "capabilities"})["vessel_id"]
        def grant(rights, accounts, connections):
            return f.request({"op": "grant", "command_id": str(uuid.uuid4()),
                "grant_id": str(uuid.uuid4()), "principal_id": str(uuid.uuid4()),
                "session_id": session, "workspace": str(f.workspace), "rights": rights,
                "accounts": accounts, "enrollment_connections": connections,
                "expires_at_ms": int(time.time() * 1000) + 120_000,
                "endpoint": "https://synthetic-vessel.invalid"})
        def scoped(credential, command, envelope=False):
            return f.request({"op": "granted", "expected_vessel_id": host,
                "grant_id": credential["grant_id"], "token": credential["token"],
                "command": command}, envelope=envelope)
        user = grant(["account_use"], [self.personal["account_id"]], [])
        catalogue = scoped(user, {"op": "accounts", "workspace": str(f.workspace), "transport": None})
        self.assertEqual([a["id"] for a in catalogue["accounts"]], [self.personal["account_id"]])
        denied_usage = scoped(user, {"op": "account_usage", "workspace": str(f.workspace),
                                    "account": self.personal, "refresh": False}, envelope=True)
        self.assertIsNotNone(denied_usage["error"])  # session grants are not private human views
        observed = f.request({"op": "account_usage", "workspace": str(f.workspace),
                              "account": self.personal, "refresh": False})
        self.assertEqual(observed["refresh_status"], "unsupported")
        self.assertIsNone(observed["snapshot"])
        self.assertEqual(catalogue["default_account"], self.personal)
        self.assertFalse(catalogue["can_set_default"])
        self.assertTrue(f.request({"op": "accounts", "workspace": str(f.workspace),
                                   "transport": None})["can_set_default"])
        forbidden = f.mutation(session, "set_account_inference", account=self.work,
            model="fixture-model", reasoning_effort=None, service_tier=None)
        result = scoped(user, {"session_id": session, **forbidden}, envelope=True)
        self.assertIsNotNone(result["error"])
        self.assertEqual(f.snapshot(session)["inference"]["account"], self.personal)
        all_accounts = f.request({"op": "accounts", "workspace": str(f.workspace), "transport": None})
        oauth = next(c for c in all_accounts["connections"] if c["transports"] == ["chatgpt_oauth"])
        enroller = grant(["account_enroll"], [], [oauth["id"]])
        limited = scoped(enroller, {"op": "accounts", "workspace": str(f.workspace), "transport": None})
        self.assertEqual(limited["accounts"], [])
        self.assertEqual([c["id"] for c in limited["connections"]], [oauth["id"]])
        # Unsupported/ungranted connection refuses BEFORE any provider authorization.
        denied = scoped(enroller, {"op": "enroll_account", "command_id": str(uuid.uuid4()),
            "enrollment_id": str(uuid.uuid4()), "workspace": str(f.workspace),
            "connection_id": self.personal["connection_id"], "alias": "forbidden", "label": "Forbidden"}, envelope=True)
        self.assertIsNotNone(denied["error"])
        f.request({"op": "revoke_grant", "command_id": str(uuid.uuid4()),
                   "grant_id": user["grant_id"], "expected_revision": 1})
        denied = scoped(user, {"op": "accounts", "workspace": str(f.workspace), "transport": None}, envelope=True)
        self.assertIsNotNone(denied["error"])
        self.assertEqual(f.provider.account_requests, [])

    def test_active_rotation_redacts_new_credential_before_stream_publication(self):
        f = self.f
        session, _ = f.new_account(self.personal)
        f.provider.file_tasks.add("rotate-during-run")
        turn = f.submit(session, "rotate-during-run")
        f.reached_provider(session, "rotate-during-run")
        f.cli("rotate-api", "--account", self.personal["account_id"], "--generation", "1",
              "--env", "ACCOUNT_PERSONAL_ROTATED", "--attest-same-identity")
        turn[0].set()
        snapshot = f.finished(session)
        self.assertEqual(f.authorization("rotate-during-run"),
                         ["Bearer synthetic-personal", "Bearer synthetic-personal-rotated"])
        public = json.dumps(snapshot)
        self.assertNotIn("synthetic-personal-rotated", public)
        self.assertEqual(snapshot["messages"][-1]["content"], "[REDACTED]")
        self.assertEqual(snapshot["inference"]["account"], self.personal)

    def test_rotation_preserves_binding_logout_refuses_without_fallback(self):
        f = self.f
        session, _ = f.new_account(self.personal)
        first = f.submit(session, "before-rotation")
        f.reached_provider(session, "before-rotation")
        first[0].set()
        f.finished(session)
        f.cli("rotate-api", "--account", self.personal["account_id"], "--generation", "1",
              "--env", "ACCOUNT_PERSONAL_ROTATED", "--attest-same-identity")
        second = f.submit(session, "after-rotation")
        f.reached_provider(session, "after-rotation")
        self.assertEqual(f.authorization("after-rotation"), ["Bearer synthetic-personal-rotated"])
        self.assertEqual(f.snapshot(session)["inference"]["account"], self.personal)
        second[0].set()
        f.finished(session)
        f.cli("logout", "--account", self.personal["account_id"])
        rejected = f.raw_command(session, f.mutation(session, "submit", prompt="must-not-fallback"))
        self.assertIsNotNone(rejected["error"])
        self.assertEqual(f.provider.count("must-not-fallback"), 0)
        self.switch(session, self.work)
        last = f.submit(session, "explicit-work")
        f.reached_provider(session, "explicit-work")
        self.assertEqual(f.authorization("explicit-work"), ["Bearer synthetic-work"])
        last[0].set()
        f.finished(session)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    BIN_DIR = args.bin_dir.resolve()
    unittest.main(argv=[__file__, *remaining])

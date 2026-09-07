#!/usr/bin/env python3
"""Two real Vessel-supervised voyages must execute concurrently in one workspace."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import sqlite3
import sys
import time
import threading
import unittest

from voyage_fixture import Fixture, wait_for


class ConcurrentVoyages(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture(BIN_DIR)
        self.addCleanup(self.fixture.close)
        print(f"\nFixture evidence: {self.fixture.root}", flush=True)
        self.fixture.start()
        self.a = self.fixture.new()
        self.b = self.fixture.new()
        for session in (self.a, self.b):
            self.assertEqual(self.fixture.snapshot(session)["workspace"],
                             str(self.fixture.workspace))

    def overlap(self, left="first", right="second"):
        fixture = self.fixture
        first = fixture.submit(self.a, left)
        fixture.reached_provider(self.a, left)
        second = fixture.submit(self.b, right)
        fixture.reached_provider(self.b, right)
        self.assertNotEqual(fixture.pids[self.a], fixture.pids[self.b])
        for session in (self.a, self.b):
            pid = fixture.pids[session]
            self.assertNotEqual(pid, fixture.supervisor.pid)
            self.assertEqual(Path(f"/proc/{pid}/exe").resolve(), fixture.binary)
        # Neither provider response has been released. Both are actually running.
        self.assertFalse(first[0].is_set())
        self.assertFalse(second[0].is_set())
        self.assertEqual(fixture.snapshot(self.a)["run"]["state"], "running")
        self.assertEqual(fixture.snapshot(self.b)["run"]["state"], "running")
        self.assertEqual(fixture.active_reservations(), {first[2]: 2, second[2]: 2})
        return first, second

    def assert_history(self, session, expected):
        fixture = self.fixture
        snapshot = fixture.snapshot(session)
        messages = [(m["role"], m["content"]) for m in snapshot["messages"]]
        self.assertEqual(messages, expected)
        # Check the durable canonical journal as well as the live projection.
        path = fixture.directory / "sessions" / session / "journal/journal.sqlite3"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as db:
            saved, = db.execute("SELECT state FROM sessions WHERE id=?", (session,)).fetchone()
        self.assertEqual([(m["role"], m["content"]) for m in json.loads(saved)["messages"]],
                         expected)

    def test_two_voyages_complete_concurrently_and_keep_exclusive_owners(self):
        fixture = self.fixture
        first, second = self.overlap()
        # Exact retries while running must not dispatch a second provider request.
        for session, submitted in ((self.a, first), (self.b, second)):
            receipt = fixture.command(session, submitted[1])
            self.assertTrue(receipt["duplicate"])
            self.assertEqual(receipt["run_id"], submitted[2])
        # One session still rejects a second mutating run while its first is held.
        rejected = fixture.raw_command(self.a, fixture.mutation(
            self.a, "submit", prompt="must-not-run"))
        self.assertIsNotNone(rejected["error"])
        # The actual persistent agent stores still exclude competing writers.
        key = hashlib.sha256(os.fsencode(fixture.workspace)).hexdigest()
        for session in (self.a, self.b):
            path = (fixture.directory / "sessions" / session /
                    "resources/completion" / key / "agents.execution.lock")
            with path.open("rb") as lease:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        # A competing process for the same session cannot steal its endpoint.
        duplicate = subprocess.run(
            [str(fixture.binary), "serve", "--directory",
             str(fixture.directory / "sessions" / self.a), "--session", self.a,
             "--incarnation", fixture.sessions[self.a], "--workspace",
             str(fixture.workspace), "--config", str(fixture.config)],
            env=fixture.env, cwd=fixture.workspace, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=10)
        self.assertNotEqual(duplicate.returncode, 0)
        self.assertEqual(fixture.command(self.a, {"op": "health"})["pid"],
                         fixture.pids[self.a])
        first[0].set()
        fixture.finished(self.a)
        self.assertEqual(fixture.snapshot(self.b)["run"]["state"], "running")
        second[0].set()
        fixture.finished(self.b)
        for session, submitted, prompt in ((self.a, first, "first"), (self.b, second, "second")):
            self.assert_history(session, [("user", prompt), ("assistant", f"answer:{prompt}")])
            self.assertEqual(fixture.command(session, submitted[1])["run_id"], submitted[2])
            self.assertEqual(fixture.provider.count(prompt), 1)
        self.assertEqual(fixture.active_reservations(), {})
        self.assertEqual(fixture.provider.count("must-not-run"), 0)
        self.assertEqual(fixture.provider.errors, [])

    def suspended(self, session):
        fixture = self.fixture
        directory = fixture.directory / "sessions" / session
        def observe():
            marker = directory / "stopped.json"
            if not marker.exists() or (directory / "runtime.sock").exists():
                return None
            evidence = json.loads(marker.read_text())
            if evidence.get("suspended") is not True:
                return None
            self.assertTrue(evidence["cleanup_observed"])
            self.assertEqual(evidence["session_id"], session)
            pid = fixture.pids.get(session)
            if pid and Path(f"/proc/{pid}").exists():
                return None
            return evidence
        return wait_for(observe, "clean suspension and retired process")

    def test_completed_turn_suspends_and_observations_do_not_wake_it(self):
        fixture = self.fixture
        turn = fixture.submit(self.a, "before-suspend")
        fixture.reached_provider(self.a, "before-suspend")
        old_pid = fixture.pids[self.a]
        old_incarnation = fixture.sessions[self.a]
        directory = fixture.directory / "sessions" / self.a
        # An unfinished provider request must keep the owner alive past idle grace.
        time.sleep(2.3)
        self.assertTrue((directory / "runtime.sock").exists())
        self.assertFalse((directory / "stopped.json").exists())
        self.assertEqual(fixture.command(self.a, {"op": "health"})["pid"], old_pid)
        turn[0].set()
        fixture.finished(self.a)
        self.assertEqual(self.suspended(self.a)["incarnation"], old_incarnation)
        revision = fixture.snapshot(self.a)["revision"]
        observations = [
            {"op": "health"}, {"op": "snapshot"},
            {"op": "history", "offset": 0, "limit": 10},
            {"op": "message_chunk", "index": 0, "offset": 0, "limit": 1024,
             "expected_revision": revision},
            {"op": "run_output", "run_id": turn[2], "offset": 0, "limit": 1024},
            {"op": "receipt", "command_id": turn[1]["command_id"]},
            {"op": "events", "after": 0, "limit": 10, "wait_ms": 100},
            {"op": "decisions"}, {"op": "controls", "section": "models"},
            {"op": "controls", "section": "policy"},
        ]
        # Concurrent observers share the startup fence without starting a runtime.
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda command: fixture.command(self.a, command), observations))
        self.assertEqual(fixture.sessions[self.a], old_incarnation)
        self.assertFalse((directory / "runtime.sock").exists())
        self.assertEqual(fixture.provider.count("before-suspend"), 1)
        self.assertEqual(fixture.active_reservations(), {})
        self.assert_history(self.a, [("user", "before-suspend"),
                                    ("assistant", "answer:before-suspend")])
        # A new turn restores the same session using a distinct process incarnation.
        next_turn = fixture.submit(self.a, "after-suspend")
        fixture.reached_provider(self.a, "after-suspend")
        self.assertNotEqual(fixture.pids[self.a], old_pid)
        self.assertNotEqual(fixture.sessions[self.a], old_incarnation)
        self.assertEqual(fixture.command(self.a, turn[1])["run_id"], turn[2])
        self.assertEqual(fixture.provider.count("before-suspend"), 1)
        next_turn[0].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        self.assert_history(self.a, [("user", "before-suspend"),
            ("assistant", "answer:before-suspend"), ("user", "after-suspend"),
            ("assistant", "answer:after-suspend")])
        self.assertEqual(fixture.provider.errors, [])

    def test_next_turn_uses_updated_binary_after_supervisor_restart(self):
        fixture = self.fixture
        first = fixture.submit(self.a, "old-executable")
        fixture.reached_provider(self.a, "old-executable")
        old_pid = fixture.pids[self.a]
        old_incarnation = fixture.sessions[self.a]
        self.assertEqual(Path(f"/proc/{old_pid}/exe").resolve(), fixture.binary)
        first[0].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        # A distinct executable path proves selection, without claiming compatibility
        # across different release schemas or changing the fixture's machine policy.
        installed = fixture.root / "updated-bin"
        installed.mkdir(mode=0o700)
        updated = installed / "voyage"
        shutil.copy2(fixture.binary, updated)
        fixture.restart_supervisor(updated)
        self.assert_history(self.a, [("user", "old-executable"),
                                    ("assistant", "answer:old-executable")])
        self.assertEqual(fixture.sessions[self.a], old_incarnation)
        second = fixture.submit(self.a, "updated-executable")
        fixture.reached_provider(self.a, "updated-executable")
        self.assertNotEqual(fixture.pids[self.a], old_pid)
        self.assertNotEqual(fixture.sessions[self.a], old_incarnation)
        self.assertEqual(Path(f"/proc/{fixture.pids[self.a]}/exe").resolve(), updated)
        second[0].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        self.assert_history(self.a, [("user", "old-executable"),
            ("assistant", "answer:old-executable"), ("user", "updated-executable"),
            ("assistant", "answer:updated-executable")])
        self.assertEqual(fixture.provider.count("old-executable"), 1)
        self.assertEqual(fixture.provider.count("updated-executable"), 1)
        self.assertEqual(fixture.provider.errors, [])

    def test_simultaneous_submissions_resume_one_exclusive_owner(self):
        fixture = self.fixture
        prior = fixture.submit(self.a, "before-race")
        fixture.reached_provider(self.a, "before-race")
        prior[0].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        old_incarnation = fixture.sessions[self.a]
        prompts = ("race-left", "race-right")
        gates = [fixture.provider.hold(prompt) for prompt in prompts]
        commands = [fixture.mutation(self.a, "submit", prompt=prompt) for prompt in prompts]
        barrier = threading.Barrier(2)
        def submit(command):
            barrier.wait(timeout=10)
            # Keep the exact pre-resume incarnation for both admissions.
            return fixture.request({"op": "forward", "session_id": self.a,
                "incarnation": old_incarnation, "command": command}, envelope=True)
        with ThreadPoolExecutor(max_workers=2) as pool:
            responses = list(pool.map(submit, commands))
        winners = [i for i, response in enumerate(responses) if response["error"] is None]
        self.assertEqual(len(winners), 1, responses)
        winner = winners[0]
        loser = 1 - winner
        self.assertIn("stale runtime incarnation", responses[loser]["error"])
        self.assertFalse(responses[loser]["outcome_unknown"])
        runtime = responses[winner]["result"]
        self.assertIsNone(runtime["error"])
        self.assertEqual(runtime["resumed_from"], old_incarnation)
        self.assertNotEqual(runtime["incarnation"], old_incarnation)
        fixture.sessions[self.a] = runtime["incarnation"]
        fixture.reached_provider(self.a, prompts[winner])
        self.assertEqual(fixture.provider.count(prompts[loser]), 0)
        # Both exact old and new receipts remain deduplicated in the new process.
        self.assertEqual(fixture.command(self.a, prior[1])["run_id"], prior[2])
        self.assertEqual(fixture.command(self.a, commands[winner])["run_id"],
                         runtime["result"]["run_id"])
        gates[winner].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        self.assert_history(self.a, [("user", "before-race"),
            ("assistant", "answer:before-race"), ("user", prompts[winner]),
            ("assistant", f"answer:{prompts[winner]}")])
        self.assertEqual(fixture.provider.count(prompts[winner]), 1)
        self.assertEqual(fixture.provider.count(prompts[loser]), 0)
        self.assertEqual(fixture.provider.errors, [])

    def test_default_start_retains_configuration_across_suspension(self):
        fixture = self.fixture
        default = Path(fixture.env["XDG_CONFIG_HOME"]) / "helm/config.toml"
        default.parent.mkdir(mode=0o700, exist_ok=True)
        default.write_text(fixture.config.read_text())
        default.chmod(0o600)
        session = fixture.new(configured=False)
        first = fixture.submit(session, "default-first")
        fixture.reached_provider(session, "default-first")
        first[0].set()
        fixture.finished(session)
        self.suspended(session)
        directory = fixture.directory / "sessions" / session
        registration = json.loads((directory / "registration.json").read_text())
        self.assertIsNone(registration.get("config_path"))
        journal = directory / "journal/journal.sqlite3"
        with sqlite3.connect(f"file:{journal}?mode=ro", uri=True) as db:
            settings, = db.execute("SELECT settings FROM process_configuration WHERE session_id=?",
                                   (session,)).fetchone()
        self.assertEqual(json.loads(settings)["config"]["model"], "fixture-model")
        # The session retains its executing-host choices even if defaults disappear.
        default.unlink()
        self.assertEqual(fixture.snapshot(session)["model"], "fixture-model")
        second = fixture.submit(session, "default-second")
        fixture.reached_provider(session, "default-second")
        second[0].set()
        fixture.finished(session)
        self.suspended(session)
        self.assert_history(session, [("user", "default-first"),
            ("assistant", "answer:default-first"), ("user", "default-second"),
            ("assistant", "answer:default-second")])
        self.assertEqual(fixture.provider.errors, [])

    def test_missing_suspension_evidence_recovers_history_without_replay(self):
        fixture = self.fixture
        title = "Recovered historical voyage"
        fixture.command(self.a, fixture.mutation(self.a, "rename", name=title))
        turn = fixture.submit(self.a, "proof-required")
        fixture.reached_provider(self.a, "proof-required")
        turn[0].set()
        fixture.finished(self.a)
        self.suspended(self.a)
        old_incarnation = fixture.sessions[self.a]
        directory = fixture.directory / "sessions" / self.a
        marker = directory / "stopped.json"
        retained = directory / "stopped.test-retained"
        marker.rename(retained)
        try:
            snapshot = fixture.snapshot(self.a)
            self.assertNotEqual(fixture.sessions[self.a], old_incarnation)
            self.assertEqual(snapshot["name"], title)
            self.assert_history(self.a, [
                ("user", "proof-required"),
                ("assistant", "answer:proof-required")])
            self.assertEqual(fixture.provider.count("proof-required"), 1)
            catalogue = fixture.request({"op": "catalogue"})
            entry = next(item for item in catalogue if item["session_id"] == self.a)
            self.assertEqual(entry["name"], title)
        finally:
            retained.unlink(missing_ok=True)

    def test_cancelling_one_voyage_preserves_the_other_and_allows_another_run(self):
        fixture = self.fixture
        first, second = self.overlap("cancel-me", "keep-running")
        fixture.command(self.a, fixture.mutation(self.a, "cancel", run_id=first[2]))
        fixture.finished(self.a, "cancelled")
        self.assertEqual(fixture.snapshot(self.b)["run"]["state"], "running")
        self.assertEqual(fixture.active_reservations(), {second[2]: 2})
        # Replaying the original cancelled command must not execute it again.
        self.assertEqual(fixture.command(self.a, first[1])["run_id"], first[2])
        resumed = fixture.submit(self.a, "new-turn")
        fixture.reached_provider(self.a, "new-turn")
        self.assertEqual(fixture.snapshot(self.b)["run"]["state"], "running")
        resumed[0].set()
        second[0].set()
        fixture.finished(self.a)
        fixture.finished(self.b)
        self.assert_history(self.a, [("user", "cancel-me"), ("user", "new-turn"),
                                     ("assistant", "answer:new-turn")])
        self.assert_history(self.b, [("user", "keep-running"),
                                     ("assistant", "answer:keep-running")])
        wait_for(lambda: not fixture.active_reservations(), "released host reservations")
        for prompt in ("cancel-me", "keep-running", "new-turn"):
            self.assertEqual(fixture.provider.count(prompt), 1)
        self.assertEqual(fixture.provider.errors, [])

    def test_both_voyages_delegate_and_use_isolated_tasks_while_running(self):
        fixture = self.fixture
        first, second = self.overlap("parent-a", "parent-b")
        children = []
        tasks = []
        for session, prompt in ((self.a, "child-a"), (self.b, "child-b")):
            task = fixture.tool(session, "todo", action="create", title=f"task:{prompt}")
            tasks.append(task["id"])
            gate = fixture.provider.hold(prompt, write_file=True)
            child = fixture.tool(session, "subagent", action="spawn", name=prompt, task=prompt)
            children.append((session, prompt, child["id"], gate))
            wait_for(lambda: fixture.provider.count(prompt) == 1, f"delegated {prompt} provider")
        # Both children reached inference while both parent requests remain held.
        for session, prompt, child, gate in children:
            self.assertFalse(gate.is_set())
            agents = fixture.command(session, {"op": "controls", "section": "subagents"})["value"]
            self.assertEqual([agent["id"] for agent in agents], [child])
            self.assertEqual(agents[0]["status"], "running")
            self.assertEqual(fixture.snapshot(session)["run"]["state"], "running")
        for _session, _prompt, _child, gate in children:
            gate.set()
        for (session, prompt, child, _gate), task in zip(children, tasks):
            result = fixture.tool(session, "subagent", action="wait", id=child)
            self.assertEqual(result, {"status": "completed", "result": {"summary": f"answer:{prompt}"}})
            self.assertEqual((fixture.workspace / f"{prompt}.txt").read_text(), prompt)
            self.assertEqual(fixture.provider.count(prompt), 2)
            todos = fixture.tool(session, "todo", action="list")
            self.assertEqual([todo["id"] for todo in todos["items"]], [task])
            fixture.tool(session, "todo", action="evidence", id=task,
                         text=f"Read {prompt}.txt and verified its exact contents: {prompt}")
            fixture.tool(session, "todo", action="status", id=task, status="completed")
            for kind, identity, disposition in (("agent", child, "incorporated"),
                                                ("todo", task, "completed_with_evidence")):
                fixture.tool(session, "completion", action="read", kind=kind, id=identity)
                readiness = fixture.tool(session, "completion", action="snapshot")
                fixture.tool(session, "completion", action="account", kind=kind, id=identity,
                             revision=readiness["revision"], fingerprint=readiness["fingerprint"],
                             disposition=disposition,
                             reason=f"Verified {prompt}.txt contents and the child's successful result.")
            readiness = fixture.tool(session, "completion", action="snapshot")
            self.assertEqual(readiness["total"], 2)
            self.assertEqual(readiness["accounted"], 2)
        first[0].set()
        second[0].set()
        fixture.finished(self.a)
        fixture.finished(self.b)
        self.assert_history(self.a, [("user", "parent-a"), ("assistant", "answer:parent-a")])
        self.assert_history(self.b, [("user", "parent-b"), ("assistant", "answer:parent-b")])
        self.assertEqual(fixture.active_reservations(), {})
        self.assertEqual(fixture.provider.errors, [])

    def test_shared_inference_accounting_waits_for_a_brief_concurrent_writer(self):
        fixture = self.fixture
        first, second = self.overlap("account-a", "account-b")
        path = fixture.root / "data/helm/inference/journal.sqlite3"
        with sqlite3.connect(path) as writer:
            writer.execute("BEGIN IMMEDIATE")
            try:
                first[0].set()
                second[0].set()
                # Simulate a short transaction from a third host-accounting
                # writer. Finishing a model response must tolerate contention.
                deadline = time.monotonic() + 0.25
                while time.monotonic() < deadline:
                    for session in (self.a, self.b):
                        self.assertEqual(fixture.snapshot(session)["run"]["state"], "running")
                    time.sleep(0.01)
            finally:
                writer.rollback()
        for session, prompt in ((self.a, "account-a"), (self.b, "account-b")):
            fixture.finished(session)
            self.assert_history(session, [("user", prompt), ("assistant", f"answer:{prompt}")])
            self.assertEqual(fixture.provider.count(prompt), 1)
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as reader:
            attempts = [json.loads(row[0]) for row in reader.execute("SELECT record FROM attempts")]
        self.assertEqual(len(attempts), 2)
        self.assertEqual({attempt["attribution"]["session"] for attempt in attempts}, {self.a, self.b})
        for attempt in attempts:
            self.assertEqual(attempt["outcome"], "completed")
            self.assertEqual((attempt["input_tokens"], attempt["output_tokens"]), (1, 1))
        self.assertEqual(fixture.active_reservations(), {})
        self.assertEqual(fixture.provider.errors, [])

    def test_simultaneous_first_runs_share_host_accounting(self):
        fixture = self.fixture
        # Unlike overlap(), neither session has opened the host inference store
        # before both submissions race to initialize it.
        with ThreadPoolExecutor(max_workers=2) as pool:
            a = pool.submit(fixture.submit, self.a, "simultaneous-a")
            b = pool.submit(fixture.submit, self.b, "simultaneous-b")
            first, second = a.result(timeout=15), b.result(timeout=15)
        fixture.reached_provider(self.a, "simultaneous-a")
        fixture.reached_provider(self.b, "simultaneous-b")
        self.assertEqual(fixture.active_reservations(), {first[2]: 2, second[2]: 2})
        first[0].set()
        second[0].set()
        for session, prompt in ((self.a, "simultaneous-a"), (self.b, "simultaneous-b")):
            fixture.finished(session)
            self.assert_history(session, [("user", prompt), ("assistant", f"answer:{prompt}")])
        path = fixture.root / "data/helm/inference/journal.sqlite3"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as reader:
            self.assertEqual(reader.execute("SELECT count(*) FROM projects").fetchone(), (1,))
            self.assertEqual(reader.execute("SELECT count(*) FROM sessions").fetchone(), (2,))
            counts = [row[0] for row in reader.execute("SELECT consumed FROM limits ORDER BY consumed")]
            self.assertEqual(counts, [1, 1, 2])
        self.assertEqual(fixture.provider.errors, [])

    def test_persistent_accounting_contention_fails_boundedly_without_replay(self):
        fixture = self.fixture
        first, second = self.overlap("busy-a", "busy-b")
        path = fixture.root / "data/helm/inference/journal.sqlite3"
        started = time.monotonic()
        with sqlite3.connect(path) as writer:
            writer.execute("BEGIN IMMEDIATE")
            try:
                first[0].set()
                second[0].set()
                fixture.finished(self.a, "failed")
                fixture.finished(self.b, "failed")
            finally:
                writer.rollback()
        self.assertLess(time.monotonic() - started, 10, "accounting contention must be bounded")
        for session, submitted, prompt in ((self.a, first, "busy-a"), (self.b, second, "busy-b")):
            self.assertEqual(fixture.command(session, submitted[1])["run_id"], submitted[2])
            self.assertEqual(fixture.provider.count(prompt), 1)
            self.assert_history(session, [("user", prompt)])
        # Dispatched inference whose usage could not be saved remains unknown.
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as reader:
            attempts = [json.loads(row[0]) for row in reader.execute("SELECT record FROM attempts")]
        self.assertEqual(len(attempts), 2)
        self.assertTrue(all(attempt["outcome"] == "unknown" for attempt in attempts))
        resumed = fixture.submit(self.a, "after-contention")
        fixture.reached_provider(self.a, "after-contention")
        resumed[0].set()
        fixture.finished(self.a)
        self.assert_history(self.a, [("user", "busy-a"), ("user", "after-contention"),
                                     ("assistant", "answer:after-contention")])
        self.assertEqual(fixture.active_reservations(), {})
        self.assertEqual(fixture.provider.errors, [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/release")
    options, remaining = parser.parse_known_args()
    if sys.platform != "linux":
        parser.error("this process regression requires Linux; other platforms are unverified")
    BIN_DIR = options.bin_dir.resolve()
    for name in ("vessel", "voyage"):
        if not os.access(BIN_DIR / name, os.X_OK):
            parser.error(f"build the executable first: {BIN_DIR / name}")
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt
    # The quality runner's termination path must still run fixture cleanups;
    # voyage processes deliberately do not share the test's process group.
    signal.signal(signal.SIGTERM, interrupted)
    unittest.main(argv=[sys.argv[0], *remaining], verbosity=2)

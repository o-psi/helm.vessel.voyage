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
import subprocess
import sqlite3
import sys
import time
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
        self.assertNotEqual(self.fixture.pids[self.a], self.fixture.pids[self.b])
        for session in (self.a, self.b):
            pid = self.fixture.pids[session]
            self.assertNotEqual(pid, self.fixture.supervisor.pid)
            self.assertEqual(Path(f"/proc/{pid}/exe").resolve(), self.fixture.binary)
            self.assertEqual(self.fixture.snapshot(session)["workspace"],
                             str(self.fixture.workspace))

    def overlap(self, left="first", right="second"):
        fixture = self.fixture
        first = fixture.submit(self.a, left)
        fixture.reached_provider(self.a, left)
        second = fixture.submit(self.b, right)
        fixture.reached_provider(self.b, right)
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

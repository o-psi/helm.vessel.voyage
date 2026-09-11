"""Offline Linux recovery of a voyage whose working directory was deleted."""
import argparse
import json
from pathlib import Path
import subprocess

from delivery_recovery import Fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    try:
        # A parent repository must never absorb commands from an empty recovery.
        subprocess.run(["git", "init", "-q", str(fixture.root)], check=True,
                       env=fixture.env)
        fixture.start()
        session = fixture.session()
        first = fixture.submit(session, "Remember our earlier conversation.")
        fixture.command(session, first)
        before = fixture.finished(session)
        fixture.suspended(session)
        old_incarnation = fixture.request({"op": "inspect", "session_id": session})["incarnation"]
        absent = fixture.submit(session, "This message was never sent.")
        fixture.workspace.rmdir()
        saved = fixture.command(session, {"op": "snapshot"})
        assert saved["messages"] == before["messages"], saved
        assert "working directory is missing" in saved["recovery_notice"], saved
        assert not fixture.workspace.exists(), "observation recreated the workspace"
        resolved = fixture.command(session, {"op": "resolve", "command_id": absent["command_id"],
                                             "original": absent})
        assert resolved["status"] == "not_admitted", resolved
        assert not fixture.workspace.exists(), "delivery resolution recreated the workspace"
        assert fixture.command(session, {"op": "receipt", "command_id": first["command_id"]})["status"] == "accepted"
        fixture.record("missing-workspace-history-and-delivery-resolution", resolved)

        second = fixture.submit(session, "Continue our conversation without the deleted files.")
        fixture.command(session, second)
        after = fixture.finished(session)
        fixture.suspended(session)
        assert fixture.workspace.is_dir()
        assert fixture.workspace.stat().st_mode & 0o777 == 0o700
        assert after["session_id"] == session
        assert after["messages"][:len(before["messages"])] == before["messages"]
        assert "recreated empty" in after["recovery_notice"], after
        assert after["access"] == before["access"] == "read-only"
        assert fixture.request({"op": "inspect", "session_id": session})["incarnation"] != old_incarnation
        assert len(fixture.provider.bodies) == 2, fixture.provider.bodies
        assert "Runtime workspace notice" in json.dumps(fixture.provider.bodies[-1])
        assert all(m["role"] != "system" for m in after["messages"])
        git = subprocess.run(["git", "-C", str(fixture.workspace), "rev-parse", "--show-toplevel"],
                             env=fixture.env, capture_output=True)
        assert git.returncode != 0, "recreated workspace discovered its parent repository"
        duplicate = fixture.command(session, second)
        assert duplicate["duplicate"] is True, duplicate
        assert len(fixture.provider.bodies) == 2, "duplicate replayed inference"
        late = fixture.command(session, absent, allow_error=True)
        assert late.get("error"), late
        fixture.suspended(session)
        fixture.record("same-voyage-continues-with-history-policy-and-git-boundary", after)

        # Missing parents are not recursively recreated. A failed startup must
        # still resolve non-admission, then accept a fresh turn after restoration.
        nested_parent = fixture.root / "nested"
        nested_parent.mkdir()
        fixture.workspace = nested_parent / "workspace"
        fixture.workspace.mkdir()
        nested = fixture.session()
        fixture.command(nested, fixture.submit(nested, "Before parent deletion."))
        fixture.finished(nested)
        fixture.suspended(nested)
        pending = fixture.submit(nested, "Try while the parent is missing.")
        fixture.workspace.rmdir()
        nested_parent.rmdir()
        failed = fixture.command(nested, pending, allow_error=True)
        assert failed.get("error"), failed
        assert not nested_parent.exists(), "recovery recursively created missing parents"
        resolved = fixture.command(nested, {"op": "resolve", "command_id": pending["command_id"],
                                            "original": pending})
        assert resolved["status"] == "not_admitted", resolved
        nested_parent.mkdir()
        fixture.command(nested, fixture.submit(nested, "Continue after parent restoration."))
        resumed = fixture.finished(nested)
        assert resumed["run"]["state"] == "completed", resumed
        fixture.suspended(nested)
        fixture.record("failed-startup-resolves-and-next-message-recovers", resumed)
    finally:
        fixture.close()


if __name__ == "__main__":
    main()

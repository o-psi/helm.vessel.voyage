"""Offline scoped Changes journey through actual Vessel-supervised Voyage processes.

Run after building vessel and voyage: python3 voyage/tests/workspace_changes.py --bin-dir target/debug
Uses a synthetic account and contacts no paid provider.
"""
import argparse
from pathlib import Path
import subprocess

from live_events_integration import Fixture
from approval_semantics import Gateway


def git(root, *args):
    subprocess.run(["git", *args], cwd=root, check=True, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path)
    args = parser.parse_args()
    if not __debug__:
        raise SystemExit("Assertions are required for this check")
    binaries = args.bin_dir.resolve()
    for name in ("vessel", "voyage"):
        assert (binaries / name).is_file(), f"Missing {name} binary"
    fixture = Fixture(binaries)
    try:
        git(fixture.workspace, "init", "-q")
        (fixture.workspace / "tracked.txt").write_text("before\n")
        git(fixture.workspace, "add", "tracked.txt")
        git(fixture.workspace, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-qm", "base")
        fixture.start()
        assert "workspace_changes" in fixture.request({"op": "capabilities"})["features"]
        assert "skills_catalog" in fixture.request({"op": "capabilities"})["features"]
        assert "workspace_file_catalog" in fixture.request({"op": "capabilities"})["features"]
        session = fixture.session()
        fixture.suspended(session)
        skill_dir = fixture.workspace / ".agents" / "skills" / "fixture"
        skill_dir.mkdir(parents=True)
        skill_file = skill_dir / "SKILL.md"
        skill_file.write_text("---\nname: fixture\ndescription: A fixture skill\n---\nPRIVATE SKILL BODY\n")
        tools = fixture.command(session, {"op": "controls", "run_id": None, "section": "tools"})
        assert any(item["name"] == "read_file" for item in tools["value"]["inventory"]), tools
        catalog = fixture.command(session, {"op": "controls", "run_id": None, "section": "skills"})
        assert catalog["value"]["can_read"] and catalog["value"]["skills"][0]["name"] == "fixture", catalog
        assert catalog["value"]["skills"][0]["path"] == str(skill_file)
        assert "PRIVATE SKILL BODY" not in str(catalog), "skill body leaked into catalogue"
        dependency = fixture.workspace / "node_modules"
        dependency.mkdir()
        (dependency / "hidden.js").write_text("dependency content")
        (fixture.workspace / "linked.txt").symlink_to(skill_file)
        files = fixture.command(session, {"op": "controls", "run_id": None, "section": "files"})
        assert "tracked.txt" in files["value"]["files"], files
        assert ".agents/skills/fixture/SKILL.md" in files["value"]["files"], files
        assert "linked.txt" not in files["value"]["files"] and "node_modules/hidden.js" not in files["value"]["files"], files
        assert files["value"]["truncated"], "skipped symlink must mark incomplete catalogue"
        assert "PRIVATE SKILL BODY" not in str(files), "file content leaked into catalogue"
        before = fixture.command(session, {"op": "snapshot"})
        identity = fixture.request({"op": "inspect", "session_id": session})
        requests = len(fixture.provider.bodies)
        (fixture.workspace / "tracked.txt").write_text("after\n")
        (fixture.workspace / "new.txt").write_text("untracked\n")
        status = fixture.command(session, {"op": "workspace_changes", "scope": "status"})
        assert status["path"] == "." and not status["truncated"]
        assert " M tracked.txt\0" in status["text"] and "?? new.txt\0" in status["text"], status
        diff = fixture.command(session, {"op": "workspace_changes", "scope": "unstaged",
                                         "path": "tracked.txt"})
        assert "+after" in diff["text"] and "new.txt" not in diff["text"], diff
        git(fixture.workspace, "add", "tracked.txt")
        staged = fixture.command(session, {"op": "workspace_changes", "scope": "staged",
                                           "path": "tracked.txt"})
        assert "+after" in staged["text"], staged
        invalid = fixture.command(session, {"op": "workspace_changes", "scope": "status",
                                            "path": "../other"}, allow_error=True)
        assert invalid["error"], invalid
        gateway = Gateway(fixture)
        try:
            history = gateway.grant(session, ["observe", "history"])
            denied = gateway.call(history, session, {"op": "workspace_changes", "scope": "status"})
            assert denied.get("error"), "History grant read executing workspace"
            denied_skills = gateway.call(history, session, {"op": "controls", "run_id": None,
                                                           "section": "skills"})
            assert denied_skills.get("error"), "History grant discovered executing-host skill paths"
            denied_files = gateway.call(history, session, {"op": "controls", "run_id": None,
                                                          "section": "files"})
            assert denied_files.get("error"), "History grant discovered executing-host filenames"
            reader = gateway.grant(session, ["observe", "workspace_read"])
            allowed = gateway.call(reader, session, {"op": "workspace_changes", "scope": "status"})
            assert allowed.get("error") is None and "?? new.txt\0" in allowed["result"]["text"], allowed
            refused = gateway.call(reader, session, {"op": "history", "offset": 0, "limit": 1})
            assert refused.get("error"), "Workspace read grant acquired conversation history"
            listed = gateway.call(reader, session, {"op": "controls", "run_id": None,
                                                    "section": "skills"})
            assert listed.get("error") is None and listed["result"]["value"]["skills"][0]["name"] == "fixture", listed
            listed_files = gateway.call(reader, session, {"op": "controls", "run_id": None,
                                                          "section": "files"})
            assert listed_files.get("error") is None and "tracked.txt" in listed_files["result"]["value"]["files"], listed_files
        finally:
            gateway.close()
        after = fixture.command(session, {"op": "snapshot"})
        observed = fixture.request({"op": "inspect", "session_id": session})
        assert observed["incarnation"] == identity["incarnation"] and observed["state"] == "suspended"
        assert after["revision"] == before["revision"] and after["total_messages"] == before["total_messages"]
        assert len(fixture.provider.bodies) == requests, "Changes read admitted inference"
        fixture.record("workspace-changes-suspended-read", {
            "session": session, "incarnation": identity["incarnation"],
            "status_paths": 2, "diff_bytes": len(diff["text"].encode()),
            "conversation_unchanged": True, "provider_requests": 0,
            "history_grant_denied": True, "workspace_read_grant_admitted": True,
            "skill_metadata_only": True,
            "file_names_only": True,
        })
        print("workspace changes process journey PASS", flush=True)
    finally:
        fixture.close()


if __name__ == "__main__":
    main()

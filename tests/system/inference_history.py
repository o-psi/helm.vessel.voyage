#!/usr/bin/env python3
"""Production CLI historical inspection against a genuine synthetic local ledger.

No provider credentials, network endpoint, inference or price data are involved.
The test-only seeder creates records through Store and assigns deterministic times.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-binary")
    parser.add_argument("--helm-bin", default=os.environ.get("HELM_BIN", "target/release/helm"))
    args = parser.parse_args()
    if args.test_binary is None:
        # Select Cargo's actual library test artifact, never a stale glob match.
        build = subprocess.run(
            ["cargo", "test", "-p", "helm", "--lib", "--all-features", "--no-run", "--message-format=json"],
            text=True, capture_output=True, timeout=600, check=True,
        )
        artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
        binaries = [item["executable"] for item in artifacts
                    if item.get("reason") == "compiler-artifact"
                    and item.get("target", {}).get("name") == "helm"
                    and "lib" in item.get("target", {}).get("kind", [])
                    and item.get("profile", {}).get("test") and item.get("executable")]
        assert len(binaries) == 1, "expected exactly one current Helm library test artifact"
        args.test_binary = binaries[0]
    binary, helm = str(Path(args.test_binary).resolve()), str(Path(args.helm_bin).resolve())
    with tempfile.TemporaryDirectory(prefix="helm-inference-history-") as directory:
        root = Path(directory)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": directory,
               "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
               "HELM_HISTORY_WORKSPACE": directory, "RUST_BACKTRACE": "0"}
        def seed(mode, **extra):
            result = subprocess.run([binary, "--exact", "inference::history_fixture::seed_history", "--nocapture"],
                                    cwd=root, env=dict(env, HELM_HISTORY_FIXTURE=mode, **extra),
                                    capture_output=True, text=True, check=True, timeout=10)
            if mode == "seed":
                return json.loads(next(line.removeprefix("HISTORY_SEED=") for line in result.stdout.splitlines() if line.startswith("HISTORY_SEED=")))
        manifest = seed("seed")
        config = root / "config.toml"
        config.write_text('provider = "openai-responses"\nmodel = "unused"\napi_key_env = "DELIBERATELY_UNAVAILABLE"\n')
        base = [helm, "--config", str(config), "--workspace", directory, "inference", "history",
                "--from", "2026-01-01T00:00:00Z", "--until", "2026-01-02T00:00:00Z"]
        def run(*extra, success=True):
            result = subprocess.run(base + list(extra), cwd=root, env=env, capture_output=True, text=True, timeout=10)
            assert len(result.stdout) + len(result.stderr) < 2 * 1024 * 1024
            assert (result.returncode == 0) == success, result.stderr
            return json.loads(result.stdout) if success else result.stderr
        original = run()
        assert original["schema_version"] == 1 and original["range_permits"] == 3
        totals = original["totals"]
        assert totals["retained_attempts"] == 3 and [totals[key] for key in ["completed", "failed", "unknown"]] == [1, 1, 1]
        assert totals["input"] == {"reported_sum": 3, "reported_attempts": 2, "missing_attempts": 1}
        assert totals["output"] == {"reported_sum": 9, "reported_attempts": 2, "missing_attempts": 1}
        assert run() == original, "a fresh CLI process changed an unchanged snapshot"
        first = run("--limit", "1")
        cursor = first["next"]
        second = run("--limit", "1", "--snapshot", cursor["snapshot"], "--offset", str(cursor["offset"]))
        assert second["next"] is None
        assert first["groups"] + second["groups"] == original["groups"]
        key = json.dumps(first["groups"][0]["key"], ensure_ascii=False)
        details = run("--group", key, "--limit", "1")
        next_detail = run("--group", key, "--limit", "1", "--snapshot", details["next"]["snapshot"], "--offset", "1")
        assert details["attempts"][0]["sequence"] < next_detail["attempts"][0]["sequence"]
        assert run("--session", manifest["session"])["range_permits"] == 2
        for group, expected in [("session", 2), ("agent", 2), ("purpose", 1), ("day", 1)]:
            assert run("--group-by", group)["matching_groups"] == expected
        run("--offset", "1", success=False)
        run("--limit", "101", success=False)
        run("--group", '{"kind":"purpose","name":"conversation"}', success=False)
        seed("report", HELM_HISTORY_ATTEMPT=manifest["ids"][0])
        run("--limit", "1", "--offset", "1", "--snapshot", cursor["snapshot"], success=False)
        assert run()["totals"]["input"]["reported_sum"] == 10
        seed("omitted", HELM_HISTORY_SESSION=manifest["session"])
        omitted = run()
        assert omitted["range_permits"] is None and omitted["scope_lifetime_omitted_details"] == 5
        assert omitted["totals"]["retained_attempts"] == 3 and omitted["scope_lifetime_permits"] == 9
        empty = subprocess.run(base[:base.index("--from")] + ["--from", "2027-01-01T00:00:00Z", "--until", "2027-01-02T00:00:00Z"],
                               cwd=root, env=env, capture_output=True, text=True, timeout=10, check=True)
        empty = json.loads(empty.stdout)
        assert empty["range_permits"] is None and empty["totals"]["retained_attempts"] == 0
        assert empty["totals"]["input"]["reported_sum"] is None
    print("Historical usage actual CLI grouping, drilldown, UTC bounds, restart, stale pages and missing coverage: PASS")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Voyage replacement-readiness evaluator.

`validate` is deterministic and suitable for CI. `live` runs representative work
through the configured Helm/provider and produces machine-readable evidence.
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "eval" / "scenarios.json"
REQUIRED_CATEGORIES = {"coding", "administration", "research", "writing", "data", "interruption"}


def load() -> list[dict]:
    scenarios = json.loads(MANIFEST.read_text())
    ids = [case["id"] for case in scenarios]
    if len(ids) != len(set(ids)):
        raise ValueError("scenario ids must be unique")
    missing = REQUIRED_CATEGORIES - {case["category"] for case in scenarios}
    if missing:
        raise ValueError(f"missing scenario categories: {sorted(missing)}")
    for case in scenarios:
        if not case.get("prompt") or not case.get("expect_output"):
            raise ValueError(f"incomplete scenario: {case.get('id')}")
    return scenarios


def run_live(scenarios: list[dict], helm: str, evidence: pathlib.Path) -> int:
    results = []
    failures = 0
    for case in scenarios:
        with tempfile.TemporaryDirectory(prefix=f"helm-eval-{case['id']}-") as raw:
            workspace = pathlib.Path(raw)
            for name, content in case.get("seed_files", {}).items():
                (workspace / name).write_text(content)
            started = time.monotonic()
            process = subprocess.run(
                [helm, "--workspace", str(workspace), "--approval", "never", "run", "--no-save", case["prompt"]],
                text=True, capture_output=True, timeout=300, env=os.environ.copy(),
            )
            combined = process.stdout + "\n" + process.stderr
            checks = {f"output:{term}": term.lower() in combined.lower() for term in case["expect_output"]}
            for name, term in case.get("expect_files", {}).items():
                path = workspace / name
                checks[f"file:{name}:{term}"] = path.exists() and term.lower() in path.read_text().lower()
            passed = process.returncode == 0 and all(checks.values())
            failures += int(not passed)
            results.append({
                "id": case["id"], "category": case["category"], "passed": passed,
                "exit_code": process.returncode, "duration_seconds": round(time.monotonic() - started, 3),
                "checks": checks, "stdout": process.stdout[-4000:], "stderr": process.stderr[-4000:],
            })
            print(f"{'PASS' if passed else 'FAIL'} {case['id']}")
    evidence.parent.mkdir(parents=True, exist_ok=True)
    evidence.write_text(json.dumps({"schema": 1, "results": results}, indent=2) + "\n")
    return int(failures != 0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate", "live"))
    parser.add_argument("--helm", default=shutil.which("helm") or str(ROOT / "target/release/helm"))
    parser.add_argument("--evidence", type=pathlib.Path, default=ROOT / "eval/evidence/latest.json")
    args = parser.parse_args()
    scenarios = load()
    if args.mode == "validate":
        print(f"validated {len(scenarios)} scenarios across {len(REQUIRED_CATEGORIES)} categories")
        return 0
    if not pathlib.Path(args.helm).is_file():
        print(f"Helm binary not found: {args.helm}; run cargo build --release", file=sys.stderr)
        return 2
    return run_live(scenarios, args.helm, args.evidence)


if __name__ == "__main__":
    raise SystemExit(main())

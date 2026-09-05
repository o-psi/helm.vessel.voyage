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
REQUIRED_CATEGORIES = {
    "coding", "administration", "research", "writing", "data", "interruption", "parallel",
    "task_management"
}


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


def output_text(value: str | bytes | None) -> str:
    # TimeoutExpired may contain bytes even when subprocess uses text mode.
    return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value or ""


def write_evidence(evidence: pathlib.Path, results: list[dict]) -> None:
    evidence.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=evidence.parent,
                                         prefix=evidence.name + ".", delete=False) as handle:
            temporary = pathlib.Path(handle.name)
            json.dump({"schema": 1, "results": results}, handle, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        temporary.replace(evidence)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def run_live(scenarios: list[dict], helm: str, evidence: pathlib.Path, timeout: float = 300) -> int:
    helm = str(pathlib.Path(helm).resolve())
    results = []
    failures = 0
    for case in scenarios:
        with tempfile.TemporaryDirectory(prefix=f"helm-eval-{case['id']}-") as raw:
            workspace = pathlib.Path(raw)
            for name, content in case.get("seed_files", {}).items():
                (workspace / name).write_text(content)
            started = time.monotonic()
            timed_out = False
            error = None
            try:
                process = subprocess.run(
                    [helm, "--workspace", str(workspace), "--access", "unrestricted", "run", "--no-save", case["prompt"]],
                    cwd=workspace, stdin=subprocess.DEVNULL,
                    text=True, capture_output=True, timeout=timeout, env=os.environ.copy(),
                )
                stdout, stderr, exit_code = process.stdout, process.stderr, process.returncode
            except subprocess.TimeoutExpired as failure:
                timed_out = True
                stdout, stderr, exit_code = output_text(failure.stdout), output_text(failure.stderr), None
                error = f"Helm exceeded scenario timeout of {timeout:g} seconds"
            except OSError as failure:
                stdout, stderr, exit_code = "", "", None
                error = f"Helm could not start: {failure}"
            combined = stdout + "\n" + stderr
            checks = {f"output:{term}": term.lower() in combined.lower() for term in case["expect_output"]}
            for name, term in case.get("expect_files", {}).items():
                path = workspace / name
                checks[f"file:{name}:{term}"] = path.exists() and term.lower() in path.read_text().lower()
            passed = exit_code == 0 and not timed_out and error is None and all(checks.values())
            failures += int(not passed)
            results.append({
                "id": case["id"], "category": case["category"], "passed": passed,
                "exit_code": exit_code, "timed_out": timed_out, "error": error, "duration_seconds": round(time.monotonic() - started, 3),
                "checks": checks, "stdout": stdout[-4000:], "stderr": stderr[-4000:],
            })
            write_evidence(evidence, results)
            print(f"{'PASS' if passed else 'FAIL'} {case['id']}")
    if not scenarios:
        write_evidence(evidence, results)
    return int(failures != 0)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate", "live"))
    parser.add_argument("--helm", default=shutil.which("helm") or str(ROOT / "target/release/helm"))
    parser.add_argument("--evidence", type=pathlib.Path, default=ROOT / "eval/evidence/latest.json")
    parser.add_argument("--timeout-seconds", type=float, default=300, help="positive per-scenario wall timeout (default: 300)")
    args = parser.parse_args()
    if not 0 < args.timeout_seconds < float("inf"):
        parser.error("--timeout-seconds must be a positive finite number")
    scenarios = load()
    if args.mode == "validate":
        print(f"validated {len(scenarios)} scenarios across {len(REQUIRED_CATEGORIES)} categories")
        return 0
    if not pathlib.Path(args.helm).is_file():
        print(f"Helm binary not found: {args.helm}; run cargo build --release", file=sys.stderr)
        return 2
    return run_live(scenarios, args.helm, args.evidence, args.timeout_seconds)


if __name__ == "__main__":
    raise SystemExit(main())

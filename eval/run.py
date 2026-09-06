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
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "eval" / "scenarios.json"
sys.path.insert(0, str(ROOT / "eval"))
from voyage_eval_io import EvidenceReader, relative, strict_json
from voyage_eval_oracle import ORACLE, count_acceptance, expectation
from voyage_eval_process import execute
REQUIRED_CATEGORIES = {
    "coding", "administration", "research", "writing", "data", "interruption", "parallel",
    "task_management"
}


def load() -> list[dict]:
    scenarios = strict_json(MANIFEST.read_bytes())
    ids = [case["id"] for case in scenarios]
    if len(ids) != len(set(ids)):
        raise ValueError("scenario ids must be unique")
    missing = REQUIRED_CATEGORIES - {case["category"] for case in scenarios}
    if missing:
        raise ValueError(f"missing scenario categories: {sorted(missing)}")
    for case in scenarios:
        if not case.get("prompt") or not case.get("expect_output"):
            raise ValueError(f"incomplete scenario: {case.get('id')}")
        if case.get("oracle") not in (None, ORACLE):
            raise ValueError("unsupported acceptance oracle")
        for name in [*case.get("seed_files", {}), *case.get("expect_files", {})]:
            relative(name)
        if case.get("oracle") == ORACLE:
            expectation(case["seed_files"])
    return scenarios


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
    for case in scenarios:
        started = time.monotonic()
        result = {"id": case["id"], "category": case["category"], "passed": False,
                  "exit_code": None, "timed_out": False, "output_limited": False,
                  "direct_process_reaped": False, "error": None, "checks": {},
                  "stdout": "", "stderr": ""}
        try:
            with tempfile.TemporaryDirectory(prefix="helm-eval-") as raw:
                workspace = pathlib.Path(raw) / "workspace"
                workspace.mkdir()
                data = pathlib.Path(raw) / "data"
                data.mkdir()
                for name, content in case.get("seed_files", {}).items():
                    path = workspace.joinpath(*relative(name))
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(content, encoding="utf-8")
                oracle = case.get("oracle")
                if oracle not in (None, ORACLE):
                    raise ValueError("unsupported acceptance oracle")
                environment = os.environ.copy()
                arguments = [helm, "--workspace", str(workspace), "--access", "unrestricted", "run"]
                if oracle:
                    expectation(case["seed_files"])
                    environment["XDG_DATA_HOME"] = str(data)
                else:
                    arguments.append("--no-save")
                arguments.append(case["prompt"])
                execution = execute(arguments, workspace, environment, timeout)
                result.update(execution)
                if execution["timed_out"]:
                    result["error"] = f"Helm exceeded scenario timeout of {timeout:g} seconds"
                elif execution["output_limited"]:
                    result["error"] = "Helm exceeded the bounded output limit"
                elif execution["cleanup_error"]:
                    result["error"] = execution["cleanup_error"]
                # Diagnostics can never satisfy an answer requirement.
                checks = {f"output:{term}": term.lower() in execution["stdout"].lower()
                          for term in case["expect_output"]}
                with EvidenceReader(workspace) as reader:
                    for name, term in case.get("expect_files", {}).items():
                        try:
                            checks[f"file:{name}:{term}"] = term.lower() in reader.read(name).decode("utf-8").lower()
                        except (OSError, ValueError, UnicodeError):
                            checks[f"file:{name}:{term}"] = False
                result["checks"] = checks
                if oracle:
                    try:
                        result["oracle_evidence"] = count_acceptance(case, workspace, data)
                        checks[oracle] = True
                    except (OSError, ValueError, KeyError, TypeError, IndexError, AttributeError, RecursionError):
                        checks[oracle] = False
                        result["oracle_error"] = "persisted counting/evidence/accounting acceptance failed"
                result["passed"] = (result["exit_code"] == 0 and result["error"] is None
                                    and result["direct_process_reaped"] and all(checks.values()))
        except OSError as failure:
            result["error"] = f"Helm could not start or evidence could not be read: {failure}"
        except (ValueError, KeyError, TypeError, RecursionError):
            result["error"] = "invalid or unsupported evaluation evidence"
        result["stdout"] = result["stdout"][-4000:]
        result["stderr"] = result["stderr"][-4000:]
        result["duration_seconds"] = round(time.monotonic() - started, 3)
        results.append(result)
        write_evidence(evidence, results)
        print(f"{'PASS' if result['passed'] else 'FAIL'} {case['id']}")
    if not scenarios:
        write_evidence(evidence, results)
    return int(any(not result["passed"] for result in results))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("validate", "live"))
    parser.add_argument("--helm", default=shutil.which("helm") or str(ROOT / "target/release/helm"))
    parser.add_argument("--evidence", type=pathlib.Path, default=ROOT / "eval/evidence/latest.json")
    parser.add_argument("--timeout-seconds", type=float, default=300, help="positive per-scenario wall timeout (default: 300)")
    parser.add_argument("--scenario", action="append", default=[], help="run or validate only the named scenario; repeat for multiple distinct IDs")
    args = parser.parse_args()
    if not 0 < args.timeout_seconds < float("inf"):
        parser.error("--timeout-seconds must be a positive finite number")
    scenarios = load()
    if args.scenario:
        selected = set(args.scenario)
        if len(selected) != len(args.scenario) or selected - {case["id"] for case in scenarios}:
            parser.error("--scenario requires distinct known scenario IDs")
        scenarios = [case for case in scenarios if case["id"] in selected]
    if args.mode == "validate":
        print(f"validated {len(scenarios)} scenarios across {len({case['category'] for case in scenarios})} categories")
        return 0
    if not pathlib.Path(args.helm).is_file():
        print(f"Helm binary not found: {args.helm}; run cargo build --release", file=sys.stderr)
        return 2
    return run_live(scenarios, args.helm, args.evidence, args.timeout_seconds)


if __name__ == "__main__":
    raise SystemExit(main())

"""Run a fixed local Terminal-Bench pilot with native Voyage on three efforts."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

TASKS = [
    "adaptive-rejection-sampler",
    "cancel-async-tasks",
    "db-wal-recovery",
    "extract-elf",
    "fix-code-vulnerability",
    "git-multibranch",
    "build-cython-ext",
    "break-filter-js-from-html",
    "log-summary-date-ranges",
    "query-optimize",
]
EFFORTS = ["medium", "high", "xhigh"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--dataset", type=Path, required=True, help="Pinned terminal-bench-2-1 checkout"
    )
    parser.add_argument(
        "--runtime", type=Path, required=True, help="Verified benchmark runtime bundle"
    )
    parser.add_argument("--broker-directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--job-name", required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists():
        raise SystemExit(
            "Refusing to overwrite an experiment; use a new output directory"
        )
    output.mkdir(mode=0o700, parents=True)
    runtime = args.runtime.resolve()
    broker = args.broker_directory.resolve()
    dataset = args.dataset.resolve()
    for name in TASKS:
        if not (dataset / "tasks" / name / "task.toml").is_file():
            raise SystemExit("Missing pinned task: " + name)
    source = output / "adapter"
    source.mkdir()
    shutil.copytree(
        Path(__file__).parent,
        source / "voyage_harbor",
        ignore=shutil.ignore_patterns("__pycache__"),
    )
    hashes = {
        p.name: hashlib.sha256(p.read_bytes()).hexdigest()
        for p in (source / "voyage_harbor").glob("*.py")
    }
    manifest = {
        "tasks": TASKS,
        "efforts": EFFORTS,
        "model": "gpt-6.1-sol",
        "trials": 30,
        "dataset_commit": subprocess.check_output(
            ["git", "-C", str(dataset), "rev-parse", "HEAD"], text=True
        ).strip(),
        "runtime": json.loads((runtime / "benchmark-build.json").read_text()),
        "adapter_sha256": hashes,
        "resource_policy": "task-declared CPU/memory/time; no override",
        "retries": 0,
        "per_effort_concurrency": 1,
        "total_concurrency": 3,
        "cost_policy": "authorized existing ChatGPT subscription; no spending cap requested; USD cost unknown",
        "status": "prepared",
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    config = {
        "job_name": args.job_name,
        "jobs_dir": str(output / "jobs"),
        "n_attempts": 1,
        "n_concurrent_trials": 3,
        "quiet": True,
        "retry": {"max_retries": 0},
        "agents": [
            {
                "name": "voyage_harbor.agent:VoyageAgent",
                "model_name": "gpt-6.1-sol",
                "n_concurrent": 1,
                "concurrency_group": "effort-" + effort,
                "kwargs": {
                    "binary_dir": str(runtime / "bin"),
                    "reasoning_effort": effort,
                },
            }
            for effort in EFFORTS
        ],
        "tasks": [{"path": str(dataset / "tasks" / name)} for name in TASKS],
        "environment": {
            "type": "docker",
            "delete": True,
            "mounts": [
                {
                    "type": "bind",
                    "source": str(broker / "socket"),
                    "target": "/run/voyage-benchmark",
                    "read_only": True,
                }
            ],
        },
    }
    config_path = output / "job-config.json"
    config_path.write_text(json.dumps(config, indent=2))
    env = os.environ.copy()
    env["PYTHONPATH"] = str(source)
    env["VOYAGE_BENCH_PROXY_TOKEN"] = (broker / "proxy-token").read_text()
    env["VOYAGE_BENCH_STATS_PATH"] = str(broker / "broker-stats.json")
    env["DOCKER_CONTEXT"] = "rootless"
    (output / "controller.pid").write_text(str(os.getpid()))
    manifest["status"] = "running"
    manifest["started_at_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(
        json.dumps({"job_name": args.job_name, "trials": 30, "output": str(output)}),
        flush=True,
    )
    with (output / "harbor.log").open("wb") as log:
        result = subprocess.run(
            ["harbor", "run", "--config", str(config_path)],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
    manifest["controller_exit_code"] = result.returncode
    manifest["status"] = "terminal"
    manifest["finished_at_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(
        json.dumps(
            {
                "job_name": args.job_name,
                "status": "terminal",
                "controller_exit_code": result.returncode,
            }
        ),
        flush=True,
    )
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())

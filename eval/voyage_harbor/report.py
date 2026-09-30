"""Summarize retained Harbor trials without treating setup errors as model failures."""

import argparse
import json
from pathlib import Path
import statistics


def collect(experiment):
    manifest = json.loads((experiment / "manifest.json").read_text())
    rows = []
    for result_path in sorted((experiment / "jobs").glob("*/*/result.json")):
        trial = json.loads(result_path.read_text())
        config = trial["config"]
        effort = config["agent"]["kwargs"]["reasoning_effort"]
        task = Path(config["task"]["path"]).name
        summary_path = result_path.parent / "agent/voyage-summary.json"
        summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
        verifier = trial.get("verifier_result") or {}
        rewards = verifier.get("rewards") or {}
        reward = rewards.get("reward")
        exception = trial.get("exception_info") or {}
        rows.append(
            {
                "task": task,
                "effort": effort,
                "reward": reward,
                "exception_type": exception.get("exception_type"),
                "canonical_completed": summary.get("completed"),
                "runtime_cleanup": summary.get("cleanup", "unknown"),
                "elapsed_seconds": summary.get("elapsed_seconds"),
                "usage": summary.get("usage", {}),
                "observed_completed_requests": summary.get(
                    "observed_completed_requests"
                ),
                "trial_id": trial.get("id"),
            }
        )
    groups = {}
    for effort in manifest["efforts"]:
        selected = [r for r in rows if r["effort"] == effort]
        graded = [r for r in selected if r["reward"] is not None]
        groups[effort] = {
            "finished": len(selected),
            "graded": len(graded),
            "solved": sum(r["reward"] == 1 for r in graded),
            "exceptions": sum(bool(r["exception_type"]) for r in selected),
            "cleanup_observed": sum(
                r["runtime_cleanup"] == "observed-runtime-cleanup" for r in selected
            ),
            "canonical_completed": sum(
                r["canonical_completed"] is True for r in selected
            ),
            "solved_and_canonical_completed": sum(
                r["reward"] == 1 and r["canonical_completed"] is True for r in selected
            ),
            "median_agent_seconds": statistics.median(
                r["elapsed_seconds"]
                for r in selected
                if r["elapsed_seconds"] is not None
            )
            if any(r["elapsed_seconds"] is not None for r in selected)
            else None,
            "observed_usage": {
                key: sum(
                    r["usage"][key]
                    for r in selected
                    if type(r["usage"].get(key)) is int
                )
                for key in (
                    "input_tokens",
                    "output_tokens",
                    "cached_input_tokens",
                    "reasoning_output_tokens",
                )
            },
            "trials_with_unknown_usage": sum(
                any(
                    type(r["usage"].get(key)) is not int
                    for key in (
                        "input_tokens",
                        "output_tokens",
                        "cached_input_tokens",
                        "reasoning_output_tokens",
                    )
                )
                for r in selected
            ),
            "observed_completed_requests": sum(
                r["observed_completed_requests"] for r in selected
            )
            if selected
            and all(type(r["observed_completed_requests"]) is int for r in selected)
            else None,
        }
    return {
        "manifest": manifest,
        "groups": groups,
        "trials": rows,
        "limitations": [
            "10-task pilot, one attempt per task/effort; not a full leaderboard score",
            "USD subscription cost is unknown; token counts describe observed completed responses",
            "Runtime build adds system certificate roots for the task-local credential relay",
            "Native platform behavior outside these Linux containers is not certified",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("experiment", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    data = collect(args.experiment.resolve())
    text = json.dumps(data, indent=2)
    if args.output:
        args.output.write_text(text + "\n")
    print(
        json.dumps(
            {"status": data["manifest"]["status"], "groups": data["groups"]}, indent=2
        )
    )


if __name__ == "__main__":
    main()

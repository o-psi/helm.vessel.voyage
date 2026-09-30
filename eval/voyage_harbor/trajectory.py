"""Convert revision-bound public Voyage messages into Harbor ATIF, without inference."""

import json
from pathlib import Path
from harbor.models.trajectories.trajectory import Trajectory


def write_trajectory(logs: Path, version: str, model: str, effort: str) -> None:
    path = logs / "public-history.json"
    if not path.exists():
        return
    history = json.loads(path.read_text())
    if (
        history.get("complete") is not True
        or len(history["messages"]) != history["total_messages"]
    ):
        raise ValueError(
            "Incomplete public history cannot become a complete trajectory"
        )
    steps, calls = [], {}
    for message in history["messages"]:
        role = message["role"]
        if role == "tool":
            call_id = message.get("tool_call_id")
            owner = calls.get(call_id)
            result = {"content": message.get("content", "")}
            if owner is None:
                # An observed unmatched result stays visible without fabricating a call.
                steps.append(
                    {
                        "step_id": len(steps) + 1,
                        "source": "system",
                        "message": message.get("content", ""),
                        "extra": {"unmatched_tool_call_id": call_id},
                    }
                )
            else:
                result["source_call_id"] = call_id
                owner.setdefault("observation", {"results": []})["results"].append(
                    result
                )
            continue
        if role not in ("assistant", "user", "system"):
            raise ValueError("Unsupported public history role")
        step = {
            "step_id": len(steps) + 1,
            "source": "agent" if role == "assistant" else role,
            "message": message.get("content", ""),
            "extra": {
                "native_message_index": message.get("message_index"),
                "interrupted_attempt": message.get("interrupted_attempt"),
            },
        }
        # Public image parts contain metadata, not exportable private image pixels.
        if message.get("parts"):
            step["extra"]["public_parts_metadata"] = message["parts"]
        if role == "assistant":
            step.update(model_name=model, reasoning_effort=effort)
            for call in message.get("tool_calls") or []:
                if call["id"] in calls:
                    raise ValueError(
                        "Duplicate tool call identity in public trajectory"
                    )
                if not isinstance(call["arguments"], dict):
                    raise ValueError("Non-object canonical tool arguments")
                step.setdefault("tool_calls", []).append(
                    {
                        "tool_call_id": call["id"],
                        "function_name": call["name"],
                        "arguments": call["arguments"],
                    }
                )
                calls[call["id"]] = step
        steps.append(step)
    if not steps:
        return
    trajectory = Trajectory.model_validate(
        {
            "schema_version": "ATIF-v1.8",
            "session_id": history["session_id"],
            "agent": {
                "name": "voyage",
                "version": version,
                "model_name": model,
                "extra": {"reasoning_effort": effort},
            },
            "steps": steps,
            "notes": "Native root public history only. Private reasoning and image pixels are excluded; subordinate private histories are not inferred or synthesized.",
            "extra": {
                "native_public_revision": history["revision"],
                "native_message_count": history["total_messages"],
            },
        }
    )
    (logs / "trajectory.json").write_text(
        trajectory.model_dump_json(indent=2, exclude_none=True)
    )

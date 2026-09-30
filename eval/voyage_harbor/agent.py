"""Harbor installed-agent adapter preserving Helm -> Vessel -> Voyage ownership."""

import json
import asyncio
import os
from pathlib import Path
import tempfile
from harbor.agents.installed.base import BaseInstalledAgent, NonZeroAgentExitCodeError
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext


class VoyageAgent(BaseInstalledAgent):
    @staticmethod
    def name() -> str:
        return "voyage"

    def __init__(
        self, *args, binary_dir: str, reasoning_effort: str = "medium", **kwargs
    ):
        if reasoning_effort not in ("medium", "high", "xhigh"):
            raise ValueError("Unsupported benchmark reasoning effort")
        self.binary_dir = Path(binary_dir).resolve()
        self.effort = reasoning_effort
        super().__init__(*args, **kwargs)
        self.version_text = (self.binary_dir.parent / "BUILD.txt").read_text()

    def version(self):
        return self.version_text.splitlines()[0].removeprefix("Development build: ")

    async def install(self, environment: BaseEnvironment) -> None:
        result = await self.exec_as_root(
            environment,
            "command -v python3 >/dev/null || (apt-get update -qq && apt-get install -y -qq python3 ca-certificates)",
        )
        if result.return_code:
            raise RuntimeError("Python setup failed")
        await self.exec_as_root(
            environment, "mkdir -p /opt/voyage/bin /installed-agent/voyage"
        )
        for name in ("helm", "vessel", "voyage"):
            await environment.upload_file(
                self.binary_dir / name, f"/opt/voyage/bin/{name}"
            )
        for name in ("relay.py", "runner.py"):
            await environment.upload_file(
                Path(__file__).with_name(name), f"/installed-agent/voyage/{name}"
            )
        result = await self.exec_as_root(
            environment,
            "cp /run/voyage-benchmark/provider-ca.crt /usr/local/share/ca-certificates/voyage-benchmark.crt && update-ca-certificates >/dev/null && "
            "printf '\n127.0.0.1 chatgpt.com\n' >> /etc/hosts && "
            "chmod 755 /opt/voyage/bin/* && /opt/voyage/bin/helm --version && /opt/voyage/bin/vessel --version && /opt/voyage/bin/voyage --version",
        )
        if result.return_code:
            raise RuntimeError("Voyage binaries incompatible with task image")

    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        token = os.environ.get("VOYAGE_BENCH_PROXY_TOKEN")
        if not token:
            raise RuntimeError("Missing scoped benchmark proxy credential")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(
                json.dumps(
                    {
                        "instruction": instruction,
                        "model": self.model_name or "gpt-6.1-sol",
                        "effort": self.effort,
                        "proxy_token": token,
                        "source": self.version_text,
                    }
                )
            )
            path.chmod(0o600)
            await environment.upload_file(path, "/installed-agent/voyage/input.json")
        try:
            result = await self.exec_as_agent(
                environment,
                "python3 /installed-agent/voyage/runner.py /installed-agent/voyage/input.json",
                timeout_sec=1900,
            )
        except asyncio.CancelledError:
            # wait_for waits for this bounded cancellation handler. A detached
            # docker exec is not Voyage cancellation. Fail outside Harbor's
            # recoverable agent-exit classes if cleanup cannot be observed.
            async def shutdown():
                stopped = await environment.exec(
                    command="python3 /installed-agent/voyage/runner.py --cleanup",
                    timeout_sec=45,
                )
                if stopped.return_code:
                    raise RuntimeError(
                        "Voyage timeout cleanup unresolved; verifier must not run"
                    )

            await asyncio.shield(shutdown())
            self.populate_context_post_run(context)
            raise
        except NonZeroAgentExitCodeError:
            self.require_observed_cleanup()
            raise
        self.require_observed_cleanup()
        if result.return_code:
            raise RuntimeError("Voyage trial failed; inspect sanitized agent summary")
        self.populate_context_post_run(context)

    def require_observed_cleanup(self) -> None:
        path = self.logs_dir / "voyage-summary.json"
        summary = json.loads(path.read_text()) if path.exists() else {}
        if summary.get("cleanup") != "observed-runtime-cleanup":
            raise RuntimeError("Voyage cleanup unresolved; verifier must not run")

    def populate_context_post_run(self, context: AgentContext) -> None:
        path = self.logs_dir / "voyage-summary.json"
        stats_path = os.environ.get("VOYAGE_BENCH_STATS_PATH")
        if not path.exists() or not stats_path:
            return
        summary = json.loads(path.read_text())
        stats = json.loads(Path(stats_path).read_text())
        records = [
            record
            for record in stats.get("usage", [])
            if record.get("session_id") == summary["session_id"]
        ]
        summary["observed_completed_requests"] = len(records)
        summary["usage_scope"] = (
            "provider-reported completed requests; failed/unfinished request usage may be unknown"
        )

        def total(key):
            values = [record.get(key) for record in records]
            return (
                sum(values)
                if values and all(type(v) is int and v >= 0 for v in values)
                else None
            )

        context.n_input_tokens = total("input_tokens")
        context.n_output_tokens = total("output_tokens")
        context.n_cache_tokens = total("cached_input_tokens")
        summary["usage"] = {
            "input_tokens": context.n_input_tokens,
            "output_tokens": context.n_output_tokens,
            "cached_input_tokens": context.n_cache_tokens,
            "reasoning_output_tokens": total("reasoning_output_tokens"),
            "cost_usd": None,
        }
        path.write_text(json.dumps(summary, indent=2))

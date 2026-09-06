# Helm

The target Helm is a TUI that connects only to local or remote Vessels. Each Vessel
supervises and exposes Voyage runtime processes, with **one session per independent
Voyage process**. See the [architecture](../docs/architecture.md).

**Current code still embeds execution in Helm.** `helm chat`, plain chat and
one-shot runs execute locally without Vessel. The TUI cannot yet multiplex
independent running voyages, and there is no `voyage` executable. See the
[current implementation](../docs/current-state.md) for capabilities and limitations.

From the repository root:

```sh
cargo install --path helm --locked
helm --help
helm config
helm doctor
helm chat
helm chat --plain
helm run "summarize the files in this workspace"
helm sessions
```

Configure a provider before sending a prompt. Native API providers do not require
Codex; API billing and subscription access are separate. Provider selection, local
endpoints, credentials, policy and remembered chat preferences are covered in
[configuration](../docs/configuration.md) and the
[example configuration](config.example.toml).

[Operations](../docs/operations.md) covers session controls, managed sessions,
foreground remote workers, diagnostics and recovery. These are current commands,
not the future Vessel/Voyage process interface. See
[security](../docs/security.md) for execution and privacy boundaries and
[development](../docs/development.md) for repository work. Automated tests and
evaluations were removed in #140; no current coverage is implied by this inventory.

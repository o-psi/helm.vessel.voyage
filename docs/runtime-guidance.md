# Runtime guidance

Helm treats model instructions as runtime configuration, not conversation data. The configured
base prompt describes durable operating behavior, while every execution generates a capability
contract from the live tool registry. Tool names, descriptions, and schemas therefore have one
source of truth.

The generated contract identifies the process as Helm, lists only currently registered calls, and
explicitly rejects capabilities inherited from a provider host. Provider adapters may reinforce
that boundary, but they must not invent a second tool catalog. `/tools` reads the registry directly
and is the deterministic operator-facing view; it does not ask the model to remember its tools.

System messages are deliberately excluded from saved sessions. On resume, Helm preserves user,
assistant, and tool history, then reconstructs current guidance. Loading also removes system
messages from older files. This prevents stale policy, disabled extensions, renamed calls, or a
provider's surrounding harness from becoming durable authority.

Tests should exercise both levels:

- structural tests prove generated guidance and persisted sessions reflect the registry;
- a live provider smoke test asks for exact call names and rejects foreign capabilities;
- behavioral evaluations require the model to select and successfully use relevant calls.

This follows the pattern used by agent runtimes that pass tools as explicit name/description/schema
objects and generate active extension context at runtime. It also accommodates Codex app-server's
current constraint that dynamic tools are supplied when a thread starts: a Helm agent's registry is
fixed for that provider thread, and a changed registry requires a new thread.

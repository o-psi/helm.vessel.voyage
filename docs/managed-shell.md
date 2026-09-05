# Managed shell cleanup

`ManagedShell` implements the usual `shell` tool with the same command policy,
approval request, workspace, environment, timeout, and output format. Managed
frontends retain a clone for the root agent and every child agent, then call
`shutdown` before acknowledging their durable cleanup obligation. The ordinary
`Shell` implementation remains available to existing unmanaged callers.

On Linux, each admitted command starts a fresh session. A bounded worker retains
the direct child without reaping it while observing that session. Output is
read from nonblocking pipes with bounded work per poll; each pipe retains at most
the configured output limit plus one byte, capped at 8 MiB plus one byte. Returned
combined output uses the usual truncation marker. A manager accepts at most 64
unobserved commands. It reclaims capacity only after observing cleanup.

A successful tool response describes the direct command's exit and output.
Background members of its session can still run until managed shutdown. Dropping
an unfinished tool future, local cancellation, or the tool timeout requests
cleanup. Dropping the last manager also requests cleanup; it does not establish
that cleanup was observed.

`shutdown` permanently closes admission, requests cancellation of every retained
command, and waits up to its caller's budget. A job admitted just before shutdown
remains tracked across process creation. Cleanup uses stable process handles and
rechecks session membership before signaling. The unreaped session leader keeps
its identity from being recycled. Observation succeeds only after no live member
of the owned session remains and the direct child has been reaped. Pipe handling
runs in the owning worker, without detached reader tasks. The worker's cleanup
attempt is bounded to five seconds; the caller can choose a shorter wait.
An unrepresentable shutdown deadline is treated as a zero wait budget. An
unrepresentable execution timeout is rejected before creating a child.

`ShellShutdown.observation_complete` is false when any job remains unconfirmed;
`remaining` contains opaque job IDs, not commands or captured text. A failed
observation retains the child handle and does not free admission capacity. The
frontend must retain its durable cleanup blocker on failure, process death, or
an insufficient wait budget. Explicit operator attestation is separate from
observed cleanup and must not imply rollback of a tool's effects.

This is application-level lifecycle observation, not an OS sandbox. A descendant
that deliberately creates a different session is outside the observed ownership
scope. Stronger process containment remains separate work. Platforms without the
Linux observation backend reject managed shell execution before spawning a child;
this slice does not claim native macOS or Windows validation.

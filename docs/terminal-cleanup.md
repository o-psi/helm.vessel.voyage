# Observable terminal shutdown

`ProcessTool::shutdown(timeout)` closes shared terminal admission before observing
cleanup. `ToolRegistry::shutdown_terminals(timeout)` forwards to this operation.
Already approved but queued starts must recheck the shared latch. Model and human
writes are rejected after shutdown; existing snapshots remain readable.

The bounded report contains only terminal UUIDs and typed failure codes, with at
most 64 remaining IDs and a separate omitted count. It contains no commands,
terminal output, human input, or subprocess diagnostic strings. Repeating shutdown
can finish observation after a timeout. Cancelling its waiter does not reopen
admission or prove cleanup; an already running blocking observer can finish later.

On Linux, completion means each direct PTY child was reaped, its original PTY
session had no live processes, and its output reader finished. Observation uses
`/proc` session membership and PID start identity. The leader remains waitable
until observation finishes, preventing session-ID reuse; other members are
signalled using stable process descriptors. Zombies outside our child relationship
are no longer executing but cannot be reaped by this process. See the Linux
[waitid contract](https://man7.org/linux/man-pages/man2/waitpid.2.html) and
[process-descriptor signalling contract](https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html).

This is not an OS sandbox. Descendants that leave the original session, remote
work, and external side effects are outside this observation scope. Missing process
identity, unavailable process observation, reader delays, poisoning, and timeouts
produce an incomplete report. Non-Linux platforms conservatively report unavailable
session observation for nonempty managers; Linux tests do not establish native
Windows or macOS behavior.

Explicit terminal close reports that termination was requested and attempts bounded
observation without closing the manager to future starts. Failed startup and close
handles remain tracked and count against the resource limit until observation
succeeds. This can conservatively retain capacity on unsupported platforms.
Ordinary destruction remains best effort and is never observed cleanup evidence.

A managed coordinator must retain a session execution fence or durable admission
blocker until cleanup is confirmed. It must retain every child-agent terminal
manager too, stop and join its watchers, and separately drain subagents and other
resource types. This helper does not perform those operations or clear a Journal
obligation. Operator acknowledgement is separate attributed recovery evidence, not
observed cleanup. The full managed frontend work remains tracked in issue #78.

# Configured MCP servers

Helm discovers tools from explicitly configured MCP stdio servers and registers
them as `mcp_SERVER_TOOL`. Servers run as ordinary child processes with a cleared,
explicitly configured environment. This transport provides no OS sandbox.
Read-only mode does not launch servers. Other modes apply external-tool policy
before invocation; initialization itself can run server code. Configure only
programs whose ordinary account authority you accept.

```toml
[mcp_servers.example]
command = "/absolute/path/to/reviewed-server"
args = ["--stdio"]
```

The current client negotiates MCP `2025-06-18`. An initialization error, malformed
result or unsupported version prevents discovery. Servers without the `tools`
capability contribute no tools. Server requests cannot invoke Helm tools: ping is
answered, and unsupported methods receive a protocol error. Server stderr is
discarded so it cannot corrupt the terminal.

Each encoded outgoing request and incoming JSON frame is limited to **1,048,576
UTF-8 bytes**, excluding its newline delimiter. The incoming limit applies while
reading, including when a peer never sends a newline. A request can consume at
most **128 unrelated messages** before its own response. Results must fit
`max_output_bytes`; an oversized result returns an explicit error. JSON escaping
counts toward the wire bound. These are Helm limits, not MCP-mandated numbers.

Initialization, discovery and tool waits use `command_timeout_secs`. Requests
to one server are serialized, and queue waiting counts toward a tool's timeout.
Cancelling a request still waiting for the serial slot sends nothing. After
possible dispatch, cancellation, timeout or a dropped caller retires that
connection: subsequent calls fail until the operator rebuilds the runtime.
Helm never automatically resends a tool call or restarts a retired server.

For a fully written ordinary request, Helm attempts `notifications/cancelled`
with the exact request ID, allowing at most 100 ms for that write. It sends no
cancellation notification for initialization, or after an incomplete request
write. Cleanup closes stdin, allows 100 ms for cooperative exit, then uses the
existing bounded process observer. The cleanup task has a five-second bound;
Linux session observation has a four-second bound. Retained resource owners can
retry unconfirmed cleanup. On other platforms, direct-child retirement does not
attest full descendant cleanup.

Cancellation delivery is not evidence that an external effect stopped. A server
may already have completed work or may ignore cancellation. Inspect the affected
system before deliberately requesting new work. Protocol failures and partial
frames also retire the connection; a late response cannot resolve a different
request. Registry and runtime ownership keep cleanup separate from the lifetime
of the cancelled wait.

Managed-session execution continues to refuse effectful MCP configurations;
these changes do not enable that workflow or supply an executable extension SDK.
See [security operations](security-operations.md) and
[managed sessions](local-managed-sessions.md) for their authority boundaries.

Protocol references: [lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle),
[cancellation](https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/cancellation),
and [stdio framing](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports).

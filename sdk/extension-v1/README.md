# Protocol-1 reference SDK (Go, standard library only)

**Source-only, uncompiled and untested while the #63 verification slot is unadmitted.** See the
[protocol and integration contract](../../docs/extension-sdk.md). No binary is
checked in; these examples have not yet demonstrated static-ELF admission or
supervised execution. Neither example imports internal Voyage Rust APIs.

- `cmd/transform`: deterministic uppercase structured input; capability `execute`.
- `cmd/read`: policy-brokered UTF-8 read of at most 4096 bytes; additionally requires
  `host.file.read`. No local-file fallback or provider/environment access.
- `cmd/conformance`: offline hostile/lifecycle fixture (crash, held pipes, flood,
  cancellation, isolation checks and structured commands); not a useful-work example.
- `sdk`: bounded private NDJSON support, compiled-in definition confirmation,
  progress, structured results, authored errors, host reads, cancel and shutdown.
- `protocol.schema.json`: language-neutral field schema. State, byte and deadline
  constraints in the contract are normative in addition to schema shape.

After **explicit release and an assigned build slot**, from this directory, a
machine with approved Go tooling can build static Linux x86_64 examples:

```sh
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o transform ./cmd/transform
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o read-text ./cmd/read
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o conformance ./cmd/conformance
```

These are future author commands, not commands executed by this delivery. Parent
packaging must inspect/validate the ELF, integrity-pin it into a format-2 archive
and copy the corresponding `definitions.json` into the manifest's `definitions`.
The SDK confirms these exact compiled-in definitions and capabilities. Do not
launch by mutable external executable path or treat a successful direct run as
sandbox evidence. Archive command spelling belongs to parent packaging docs.

Typical private exchange (UUIDs abbreviated here for readability, not runnable):

```text
host: initialize protocol=1 identity=(session,incarnation,run,invocation,package,digest) definitions=<pinned> capabilities=[execute]
child: initialized protocol=1 identity=<exact echo> definitions=<compiled-in pin> capabilities=[execute]
host: invoke invocation=<same> kind=tool name=uppercase arguments={"text":"hello"}
child: progress invocation=<same> text="Transforming text"
child: result invocation=<same> value={"text":"HELLO"}
host: shutdown
child: shutdown_ack
child: <stdout EOF, process exit>
host: <separate positive process-tree cleanup observation or durable pending blocker>
```

This is an illustrative transcript, **not recorded run evidence**. The read
example additionally sends `host.file.read` and awaits `host_result`/`host_error`
before emitting another message. A read error returns the fixed `failed` code;
no host error details are exposed. SDK has no cancellation ACK: cancellation
stops admission/output and the host proceeds to shutdown and observed cleanup.
Use the provided context for cooperative handlers; the host deadline remains
authoritative even when a handler ignores it. Applications must exit after
`Serve` returns; this process-oriented helper is not a reusable persistent server.

# Shared composer drafts

Helm clients edit unsent composition; **Vessel owns durable shared drafts**. Drafts
are not conversations, sessions, run queues or execution reservations. Saving a
new-chat draft must not start a Voyage. Provider credentials stay on the executing
host and are never draft fields.

## Identity and authority

A draft has a UUID, a monotonically increasing revision, an update time, an ordered
list of text/image parts and an explicit target:

- `new_chat`: a workspace, without a session;
- `message`: an existing session;
- `steer`: an existing session, process incarnation and run ID.

The local Vessel owner and separately paired full-access owner clients share the
owner namespace. A scoped connection or process grant has an isolated namespace;
it does not gain visibility into the owner's unfinished messages. Existing grant
rights, workspace/session scope, expiry and revocation remain enforced. Connecting
to another Vessel never transfers a draft implicitly.

Discovery is server-side: the phone does not need a UUID stored only on the TUI's
machine. Multiple new-chat drafts retain distinct identities. A client's local
unsaved recovery copy is not proof that the Vessel saved it, and canonical
conversation history is never used as a draft store.

## Editing and admission

Autosave uses compare-and-swap revisions, not last-writer-wins. Clients must retain
local edits on an offline save or conflict and show the actual saving/saved/error
state. A stale client cannot overwrite newer composition without explicit
reconciliation. Loading a draft does not send it. In particular, a steering draft
is tied to its reviewed run and must not silently become steering for a later run.

Sending and saving are different operations. The client preserves the immutable
send identity before dispatch and reconciles uncertain admission through its
receipt. Only an observed accepted send permits clearing the sent draft revision;
a concurrent newer edit must survive. Rejected or unresolved sends retain the
draft. Pictures currently use multimodal **submission**, not text-only steering;
clients must refuse picture steering without discarding its text or attachments.

## Pictures

Draft images use a private Vessel staging namespace, including before a new voyage
exists. Accepted types are PNG, JPEG and WebP, validated from raster bytes rather
than trusting a filename or browser MIME declaration. The existing bounded raster
decoder rejects malformed containers, unsupported animation and excessive
resource use. Images do not become remote URLs, filesystem paths or base64 prompt
text.

The draft holds immutable image metadata and ordered references. Authorized
chunked reads supply preview bytes. Before submission, explicit promotion copies
staged images through the destination Voyage's existing upload operation and
returns session-owned content references. Promotion does not submit, retarget or
clear a draft. Deleting a draft must not delete promoted conversation artifacts.
The selected model's image-input capability is checked by submission; rejection
must leave composition available for recovery.

Web previews and history use authorized bytes in local object URLs rather than
public artifact URLs. Untrusted image labels are text, never HTML. Browser/mobile
file selection, paste and drag/drop are UI entry points to the same validated
upload path, not independent authority surfaces.

## Public contract

Draft operations use the existing authenticated Vessel command transport:

```json
{"protocol":1,"command":{"op":"drafts","operation":{"op":"list"}}}
```

`list` returns `{"drafts":[...]}`; `get` returns the current draft or null.
`put` supplies `command_id`, `draft_id`, `expected_revision` and `document`;
revision zero creates a previously unused draft identity. `delete` supplies the
same identities and the exact revision to clear. Reusing a mutation identity with
a different payload is refused.

`upload_image` supplies a draft ID, stable command ID, display name and bounded
base64 bytes. It returns the standard `ImageAttachment` metadata. `read_image`
supplies the draft/attachment IDs, offset and bounded limit, returning metadata,
base64 bytes and `next_offset`. `promote` supplies a draft ID, its exact revision,
a destination session ID and stable command ID; its result is `{"parts":[...]}`.
All these operations remain subject to current Vessel authority checks.

## Verification boundaries

Focused Rust and Web tests exercise the contract and client behavior. The offline
Linux process fixture is:

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 vessel/tests/drafts_workflow.py --bin-dir target/debug
```

It uses synthetic pixels and independent local HTTP clients, not real phones or
live providers. Fixture success does not establish native macOS/Windows security,
a deployed TLS/OAuth journey, browser file-picker behavior on a physical phone or
live-provider image quality. See [validation](quality.md),
[Helm Web](helm-web.md), [process access](process-access.md) and
[architecture](architecture.md) for the surrounding boundaries.

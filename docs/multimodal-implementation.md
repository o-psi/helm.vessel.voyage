# Images and screenshots

Implemented for [issue #74](https://github.com/o-psi/voyage/issues/74). Image input
uses Helm's connected TUI, public Vessel routing, and a separately supervised
Voyage. This is native multimodal input, not an image path embedded in a prompt.

## Operator workflow

1. In an existing-voyage or new-voyage composer, press **F6** (or **Ctrl+I** on
   terminals that distinguish it from Tab).
2. Enter a local PNG, JPEG or WebP path and press Enter. Paths containing spaces
   need no shell quoting. Files are read on the **Helm host**, including when the
   executing Vessel is remote. The modal shows name, actual media type, byte size
   and dimensions. Enter `remove INDEX` to remove a displayed attachment.
3. Escape returns to the composer without changing its text. Enter sends text and
   attached images; a completely image-only first turn is supported. The composer
   sends text followed by images in attachment order. The public API also supports
   arbitrary ordered text/image interleaving.

Draft bytes are retained privately: changing or removing the original file does
not change the draft. A pending image send freezes the complete draft. Reconnecting
resolves its original command ID; it never replays an uncertain submission or
resumes uploads automatically. Rejection retains the draft for correction and a
new explicit send. Image steering during an active run is refused without consuming
text or attachments; wait for completion before sending images.

### Screenshots

Enter `screenshot` in the image modal, review the full-display privacy warning,
and type **CAPTURE** to confirm. Escape cancels the confirmation. Capture adds an
image to the draft only; sending remains a separate action. Visible secrets cannot
be removed by text redaction, so review the screen before confirming.

Capture is Helm-local and uses its invocation configuration when present, otherwise
its local default configuration. It rechecks local policy, refuses read-only mode
and denied commands, and refuses required process isolation rather than bypassing
it to access the desktop. It never uses a remote runtime's credentials or shell.
Linux Wayland uses `/usr/bin/grim`; Linux X11 uses `/usr/bin/maim`. A missing utility,
missing display, desktop refusal, or unsupported platform produces an actionable
error without changing the draft. Non-Linux automatic capture is unsupported.

Capture uses fixed arguments, suppresses subprocess diagnostics, bounds output and
CPU/memory, restricts inherited environment to desktop variables, and observes child termination on failure or timeout. The UI currently
waits synchronously for at most the five-second capture window; Escape cancellation
is available **before** capture, not during that window. No capture is automatic.

## Limits

| Boundary | Limit |
|---|---|
| Supported files | Single-frame PNG, JPEG, WebP; actual signature and full raster verified |
| One image / total images in a turn | 2 MiB / 2 MiB |
| Images / ordered parts per turn | 4 / 16 |
| Authored turn text | 64 KiB |
| Dimensions | At most 8192 on either axis and 16,777,216 pixels |
| Decoded raster buffer | At most 64 MiB; decoder bookkeeping is additional |
| Progressive JPEG work | At most 32 scans |
| Retained image occurrences in one provider request | 4, totaling at most 2 MiB; repeated references count again |
| Serialized provider request | 8 MiB including tools, text escaping and encoded images |
| Public and private wire envelope | Existing 4 MiB limit, unchanged |
| Private image database | 64 MiB including database overhead; at most 128 immutable uploads |
| Private Helm draft | 8 MiB |

If retained images exceed a request limit, compact older image turns or start a new
voyage. Unreferenced uploads are bounded by the session store limit and retained
until session deletion; removing a draft attachment does not delete a blob already
uploaded. Clear/compact does not silently reclaim blobs needed by retained evidence.

## Transport, admission and storage

`UploadImage` transfers a bounded canonical-base64 file through authenticated Vessel
routing. It requires Execute authority on both public and private routes. Voyage
fully decodes and verifies the raster and stores it in a private session-bound
SQLite database under `journal/images/`. A single SQLite transaction either retains
an entire upload or retains none; no partially finalized blob is exposed. Repeating
an upload UUID is valid only for exactly the same principal, name and bytes.

`SubmitContent` contains only ordered text and authoritative image references.
References include UUID, SHA-256, name, verified type/size/dimensions, never a path
or image bytes. The executing owner resolves references within the addressed
session and verifies bytes, digest and metadata before new admission. Cross-session,
missing and corrupt references are refused explicitly.

This split keeps image bytes out of the 128 KiB command-reservation journal. Existing
text-only Submit/Steer wire encoding and turn digests are unchanged. Ordered content
and immutable image identity participate in exact command binding and admission.
An existing exact receipt is resolved before current-model or expiry checks; changing
content under an existing command ID is not a retry. Older peers reject the added
operations instead of silently dropping images. Runtime health advertises
`upload_image` and `submit_content`.

Canonical checkpoints preserve ordered references. Provider hydration is transient
and excluded from serialization and Debug. Resume reopens the private store;
branch creation copies verified bytes into the destination's own store before
publishing its snapshot. Deletion removes the image store before reporting observed
cleanup. Ordinary history, transcripts, exports and diagnostics carry metadata only.

Journal schema **9** prevents an older text-only runtime from opening and silently
rewriting image history. An idle v8 owner promotes its journal under the execution
fence when images are first used; older schemas require the existing explicit
quiescent upgrade. Image-bearing owner transfer and legacy JSON-store branching
are refused explicitly until they can retain blobs safely. Use Vessel branching
for image-bearing voyages. Outbound enrollment relay execution is a separate
surface and does not expose these image operations.

## Providers and privacy

Native OpenAI Chat Completions uses ordered `text`/`image_url` blocks. Responses
and native ChatGPT OAuth use `input_text`/`input_image` blocks. Anthropic uses
`text` and base64 `image` source blocks. The optional Codex compatibility bridge
refuses images before dispatch.

Capability admission uses exact selected-model metadata, with a conservative exact-ID
fallback for known image-capable models when modalities are absent. Explicit text-only
metadata overrides that fallback; unknown models fail closed with guidance. Native
adapters refresh capability metadata before image-bearing dispatch, including retained
history and continuation. Supported wire format is not a claim of account entitlement.

Both initial provider failures and errors arriving later in a stream omit provider
error text on image-bearing requests, including errors that echo base64. Configured
secrets in text parts are redacted; secrets split across parts cause refusal rather
than leakage. Pixel content itself is never text-redacted. Private image stores and
private draft files must not be copied into ordinary diagnostic bundles.

Wire formats were checked against the official
[OpenAI image guide](https://platform.openai.com/docs/guides/images-vision) and
[Anthropic vision guide](https://docs.anthropic.com/en/docs/build-with-claude/vision).

## Verification

Targeted checks are local and use synthetic images and loopback providers:

```sh
cargo test -p voyage-protocol --locked content:: --lib
cargo test -p voyage --locked --lib
cargo test -p helm --locked --lib
cargo clippy -p helm -p vessel -p voyage -p voyage-protocol --all-targets --locked -- -D warnings
cargo build -p helm -p vessel -p voyage --locked
python3 voyage/tests/images_workflow.py --bin-dir target/debug
python3 voyage/tests/images_composer.py --bin-dir target/debug
```

The unit checks exercise PNG/JPEG/WebP decoding, truncation (including fake JPEG
terminators), oversized dimensions, animation refusal, corrupt checksums, file
symlink/FIFO refusal, immutable storage conflicts, reopening and branch copies,
serialization/Debug privacy, provider encoding, capability refusal, aggregate and
serialized request limits, error sanitization, composer removal and recovery,
upload-before-submit ordering, and screenshot confirmation/output/timeout handling.

The supervised workflow check exercises authenticated uploads, cross-session refusal,
image-only admission, exact duplicate/conflicting commands, ordered content, retained
images across process suspension, unsupported models before dispatch, branch image
retention, and provider-error privacy through actual Vessel and Voyage processes.

The controlling-PTY composer check exercises F6 add/remove, metadata display,
text preservation, private-file permissions, restart recovery after deleting the
source image, text+image and image-only first sends, existing-session sends,
screenshot confirmation cancellation, and observed child cleanup. Unit fixtures
also exercise uncertain-send recovery without resubmission.

These are **offline Linux** checks, not paid/live-provider certification, a deployed
HTTPS-route test, or native macOS/Windows evidence. Automatic desktop capture is not
exercised against the operator's real screen by these checks; subprocess fixtures
verify bounded capture and cleanup without collecting personal pixels.

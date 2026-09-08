# Images and pasted screenshots

Image input is part of the **normal Helm composer**, not a dialog. The input path
is local Helm acquisition → authenticated Vessel upload → private Voyage storage
→ ordered provider content. See [issue #74](https://github.com/o-psi/voyage/issues/74).

## Paste into the input box

1. Copy an image, or take a screenshot with the operating system's screenshot tool.
2. In either a new-voyage or existing-voyage composer, press **Ctrl+V** or **Alt+V**.
   A delivered Shift+Insert key also invokes paste.
3. The image appears as an owned **`[Image N]`** element at the cursor. Continue
   typing before or after it. The composer also shows name, actual type, byte size
   and dimensions in ordinary rows, without replacing the conversation.
4. Press Enter to send. Image-only first messages work.

Left/Right cross image elements atomically. Backspace/Delete removes an adjacent
owned element and its private draft attachment without deleting surrounding text.
Typing the literal characters `[Image 1]` does **not** create an attachment. Image
numbering follows document order. Sends retain true text/image interleaving; marker
labels themselves are not inserted into the model's authored text.

You can also paste an exact local image path, a quoted/escaped path containing
spaces, or local `file://` URLs. Multiple explicit paths/URI-list entries are bounded
by the same four-image limit. Terminal drag-and-drop works when the terminal sends
usable path text. Missing paths remain text; existing malformed/unsafe raster files
fail without replacing the draft. HTTP(S) URLs are not downloaded and ordinary
prose is not treated as an attachment. Relative raster paths and `~/` paths are
supported; WSL drive conversion assumes `/mnt/<drive>`. UNC strings are not mapped
to guessed Unix network mounts.

**There is no image modal, F6 attachment workflow, `remove INDEX` command, or
in-app `screenshot`/CAPTURE flow.** Capturing the screen is an OS action; pasting an
already captured image does not capture anything new or require another capture
confirmation.

**Alt+P toggles optional inline previews** of already-owned composer images.
Metadata remains available when previews are disabled, the terminal uses text
fallback, or an image exceeds the preview-only decoding limits. Toggling does not
capture, upload, submit, or modify the saved image. See
[Composer image previews](ui-previews.md) for terminal protocol selection,
resource bounds and verification limitations.

### Terminal paste versus native clipboard access

Ctrl+V/Alt+V are application clipboard actions when the terminal delivers those
keys. Ctrl+Shift+V, Cmd+V or a terminal's Paste menu may instead send bracketed
**text** paste. Helm handles that text directly and recognizes image paths in it.
An exactly empty bracketed-paste event attempts native clipboard acquisition.
Some terminals emit no event at all for an image-only clipboard; use Alt+V in that
case. Normal nonempty text paste never reads an unrelated clipboard image.

Clipboard reads happen on the **Helm host**, even when the executing Vessel is
remote. Running Helm itself inside SSH does not magically forward the SSH client's
image clipboard: use a local Helm connected to the remote Vessel, a deliberately
configured clipboard/display bridge, or a path accessible to the Helm process.

## Responsive acquisition and recovery

Native reads run asynchronously. You can keep typing while a read completes; a
left-biased editor anchor preserves the requested insertion location and rebases
with subsequent edits. **Escape cancels** acquisition and retains the draft.
Replacing/discarding the whole draft invalidates the pending insertion instead of
pasting into a different message. A read pins its original voyage/draft identity.
Sending that draft is refused until acquisition finishes or cancellation completes.
Closing Helm cancels and waits for its clipboard reader; it does not cancel a voyage.

Validated bytes are saved privately, not left dependent on the source clipboard
or file. Drafts recover even if the original image disappears. Older text-plus-image
records are migrated to owned inline elements; their immutable pending commands,
IDs, image bytes and SHA-256 references are not rewritten. A pending image send
freezes the complete draft. Reconnect checks the original command identity, without
replaying uncertain submission or uploads. Rejection keeps the complete editable
draft for a new explicit send.

Plain prompt-history recall retains authored text, not image ownership. It is
suppressed while the current composer owns images so it cannot silently replace
an image draft. Durable draft/resume recovery retains the actual images.

## Native clipboard backends

| Host | Acquisition |
|---|---|
| Linux Wayland | `wl-paste` MIME discovery, then PNG/JPEG/WebP, file URI lists or UTF-8 text |
| Linux X11 | `xclip` TARGETS discovery with the same supported content categories |
| macOS | `osascript` PNG clipboard conversion into unique private temporary storage; `pbpaste` text fallback |
| Windows / WSL | Static noninteractive PowerShell for clipboard file lists, bounded PNG output or text |

Linux supports `text/uri-list` and GNOME copied-file lists; pasting a cut-file list
does not move or delete the original files. Unknown/empty clipboard contents,
missing helpers, denied commands, invalid data, overflow and timeouts produce a
visible error while retaining existing text and images. Install **wl-clipboard**
for Wayland or **xclip** for X11 when needed.

Acquisition is an explicit local **input** operation and works in read-only mode;
it does not grant the model permission to run clipboard tools. It rechecks local
policy and command denials. Required process isolation refuses native helper access
rather than silently bypassing desktop isolation. Helper environments contain only
needed connection/platform variables, never provider credentials or loader injection
variables; stderr is suppressed. Helpers have one shared five-second deadline,
bounded output, and observed kill/reap cleanup on cancellation, overflow or timeout.
No clipboard is read in the background or on ordinary text paste.

Linux fixtures establish the tested behavior below. macOS/Windows/WSL backend code
is present, but this work does **not** claim native platform certification or remove
existing platform restrictions elsewhere in Helm/Vessel private storage and transport.

## Limits

| Boundary | Limit |
|---|---|
| Supported images | Fully decoded single-frame PNG, JPEG, WebP; signatures override extensions |
| Image bytes / total images per turn | 2 MiB / 2 MiB |
| Images / ordered content parts per turn | 4 / 16 |
| Composer / authored text | 64 KiB |
| Dimensions | At most 8192 per axis and 16,777,216 pixels |
| Decoded raster output | At most 64 MiB; decoder bookkeeping is additional |
| Progressive JPEG work | At most 32 scans |
| Clipboard metadata / text/file lists | 32 KiB / 64 KiB |
| Clipboard helper lifetime | One shared five-second deadline across fallbacks |
| Images retained in one provider request | 4 occurrences, totaling at most 2 MiB; duplicates count again |
| Final serialized provider request | 8 MiB including escaping, tools and encoded images |
| Public/private transport envelope | Existing 4 MiB limit |
| Private image database | 64 MiB including overhead; at most 128 immutable uploads |
| Private Helm draft | 8 MiB |

If retained images exceed a request limit, compact older image turns or start a new
voyage. Removing an inline element removes it from the draft, not from an already
accepted turn. Uploaded blobs remain bounded and retained until session deletion;
clear/compact does not silently reclaim evidence-bearing blobs.

## Transport, persistence and providers

`UploadImage` independently deduplicates bounded bytes by upload UUID, principal,
name and exact contents. Execute authorization is enforced on both public Vessel
and private runtime routes. Voyage verifies the signature, full raster, dimensions,
pixel count and size before atomically retaining the private blob.

`SubmitContent` carries ordered text and image references: UUID, SHA-256, label,
actual media type, size and dimensions. There are no file paths or image bytes in
turn command reservations. The owner resolves references only within the session
and refuses missing/corrupt/cross-session images. Existing text-only wire encoding
and digests remain unchanged. Exact receipts resolve before current-model or expiry
checks; changing content under an existing command ID is not a retry.

Canonical checkpoints retain ordered references; hydration is transient and excluded
from serialization and Debug. Resume reopens the private store; Vessel branching
copies verified bytes into the destination before publishing its snapshot. Deletion
removes the image store before reporting observed cleanup. Ordinary observations,
transcripts, exports and diagnostic records contain no image bytes.

Journal schema 9 prevents older text-only runtimes from silently rewriting image
history. An idle v8 owner promotes its schema under its execution fence; older
schemas require the explicit quiescent upgrade. Image-bearing owner transfer,
legacy JSON-store image branching, and the separate outbound enrollment relay are
not enabled by these public image operations and refuse unsupported paths.

Native OpenAI Chat Completions uses `text`/`image_url`; Responses and native ChatGPT
OAuth use `input_text`/`input_image`; Anthropic uses text and base64 image-source
blocks. Codex compatibility refuses image input. Exact selected-model capabilities
are checked before admission and image-bearing dispatch, including retained history;
unknown/explicitly text-only models fail closed. Image steering during an active
run is refused with the draft intact.

Initial and streamed errors for image-bearing provider requests omit provider error
text, including echoed base64. Configured secrets in text parts are redacted; secrets
spanning parts cause refusal. Pixel contents cannot be text-redacted, so review what
you copy before pasting/sending. Private drafts/blob stores must not be included in
ordinary diagnostic bundles.

## Verification

```sh
cargo test -p helm -p voyage -p voyage-protocol --lib --locked
cargo clippy -p helm -p vessel -p voyage -p voyage-protocol --all-targets --locked -- -D warnings
cargo build -p helm -p vessel -p voyage --locked
python3 voyage/tests/images_composer.py --bin-dir target/debug
python3 voyage/tests/images_workflow.py --bin-dir target/debug
# Repeat the supervised workflow with --provider openai-responses,
# --provider anthropic, and --provider chatgpt-oauth for the other native fixtures.
```

Unit checks cover raster validation/storage, provider encoding/privacy, atomic inline
editing, literal lookalikes, caret anchors, ordered sends, legacy draft migration,
clipboard parsing/output bounds/cancellation, and no-replay dispatch recovery.
Real controlling-PTY checks use private fake clipboard helpers and synthetic pixels:
no access to the operator's actual clipboard or display is required. They exercise
normal/direct paste, typing during acquisition, keyboard deletion, file paths,
private restart recovery, image-only/existing-session sends and failure preservation.
Supervised provider fixtures cover authorization, exact deduplication, ordered content,
resume/branch retention, limits, unsupported models and error privacy.

These checks are offline Linux evidence, not paid/live-provider certification,
real desktop-clipboard testing, deployed HTTPS verification or native macOS/Windows
validation.

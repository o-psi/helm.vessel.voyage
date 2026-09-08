# Image attachments: implementation work

Status: initial foundation for [issue #74](https://github.com/o-psi/helm.vessel.voyage/issues/74).
This is planned integration, not an available operator workflow.

## Implemented foundation

`crates/voyage-protocol/src/content.rs` defines ordered text and image references,
an explicit raster media-type allowlist, and metadata validation. Limits are supplied
by the caller, so the runtime can narrow local limits for the selected provider.
The validator preserves authored text, permits image-only input, rejects unsupported
image input, counts repeated references toward aggregate limits, and rejects unsafe
display labels. References carry neither paths nor image bytes.

This module is not connected to admission or persistence. Its checks do not establish
that files exist, are authorized, match their metadata, or can be safely decoded.

## Integration sequence

1. Add bounded image ingestion and managed storage in Voyage. Verify signatures,
   decoded dimensions, pixel budget and actual byte counts before admission. Reject
   malformed, truncated and unsupported images with bounded decoder work. Authorize
   references within the owning session; do not interpret display names as paths.
   Retain managed files across resume and branch operations, and track incomplete
   writes and cleanup obligations. Use the existing private-storage primitives.
2. Extend both `crates/voyage-protocol/src/vessel.rs` and
   `crates/voyage-protocol/src/process/types.rs`, their codecs, and
   `vessel/src/process/api.rs`. Choose an explicit bounded upload contract: the
   current public body limit is 4 MiB, so adding base64 to an unrestricted command
   is not sufficient. Preserve existing text-only payload compatibility and version
   negotiation. A remote runtime must receive bytes rather than read a Helm-local
   path. Bound decoded and serialized request sizes separately.
3. Connect `voyage/src/server/submission.rs` and canonical journal admission to the
   ordered parts. Exact deduplication must cover text, order and immutable image
   identity. Resolve image capability and validate bytes before dispatch; repeat
   capability checks when changing models or replaying retained history. Unknown
   capability must fail with actionable guidance rather than silently drop images.
4. Extend `voyage/src/model.rs`, session persistence and context handling. Preserve
   order through checkpoint, resume, branch and provider continuation. Keep binary
   content outside ordinary history observations, logs and diagnostic bundles.
   Report missing retained attachments explicitly; do not substitute empty text.
5. Implement native encoders in `voyage/src/provider/openai.rs`,
   `voyage/src/provider/openai_responses.rs` and
   `voyage/src/provider/anthropic.rs`, plus the ChatGPT OAuth path. Verify current
   provider contracts from official documentation before implementation. Treat the
   optional Codex compatibility bridge separately and explicitly reject unsupported
   attachment modes. Sanitize provider errors that might echo submitted image data.
6. Extend Helm's existing-session and new-voyage composers, persisted drafts and
   uncertain-send recovery. Show sanitized name/type/size, preserve text on removal
   or failed send, and support multiple images and image-only first send. Keep the
   immutable pending payload across uncertain delivery; editing it needs a distinct
   command identity. Include explicit screenshot capture with local policy checks,
   supported-platform detection, bounded capture and clear refusal/cancellation.
7. Verify the complete supervised workflow and document available commands in the
   operator guides only once implemented. Keep #74 open until its acceptance is met.

## Verification

The focused protocol tests cover ordered serialization, exact text preservation,
image-only turns, unsupported capability, aggregate byte accounting and overflow,
unsafe metadata, pixel limits and strict media types/fields. Run:

```sh
cargo test -p voyage-protocol --locked content::tests
```

These tests do not cover encoding actual images, decoding malicious files, provider
errors, persistence, screenshot capture or TUI behavior. Integration needs offline
provider fixtures and supervised process checks for those paths, plus actual native
evidence for any macOS/Windows claims. Live provider work needs an approved account
and budget. Existing unrelated suites need not be recreated for this feature.

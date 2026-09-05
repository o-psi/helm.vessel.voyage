# Release engineering

Voyage publishes `helm` and `vessel` together from one versioned commit. Tags use
`vMAJOR.MINOR.PATCH`. A release is immutable: corrections receive a new tag.

Push and pull-request quality checks run on Linux only to limit development costs.
The tag-triggered release workflow builds archives for Linux x86-64, macOS
x86-64/ARM64, and Windows x86-64; it does not run the workspace test suite on
macOS or Windows. Record separate platform testing before claiming validation.
Archives contain Helm, Vessel and the mock setup-preview executable, Helm and Vessel manpages, shell completions,
the README, and a SHA-256 checksum. Linux builds can be reproduced locally with:

```sh
cargo build --workspace --release --locked
./scripts/package-release v0.1.0
(cd dist && sha256sum -c *.sha256)
```

Before tagging, update versions and changelog, run the full CI command set, complete
the live evaluation suite, inspect generated manpages/completions, and test a clean
archive install. After tagging, compare artifact checksums, smoke-test both binaries
from each archive, attach evaluation/cutover evidence, and publish known limitations.

The workflow uploads build artifacts but intentionally does not auto-publish or sign
a release. Add repository-specific signing and Forgejo release credentials only after
the project establishes its key custody and release-approval policy.

Unix builds also package a separate compressed `voyage-installer` with
`scripts/package-installer`. See [setup preview](installer-preview.md) for asset
names and local usage. The small `install.sh` launcher requires these files to be
published as release assets before remote download works; it does not download the
full Voyage archive. Keep the wizard explicitly marked as mocked until real
provisioning is implemented and verified.

## Connectivity transition release note

This build intentionally removes legacy pairing/task-worker connectivity before
`helm attach` is implemented. Include [retirement and data-preservation guidance](vessel-connectivity-retirement.md)
in the release notes; do not advertise remote readiness from successful local tests.

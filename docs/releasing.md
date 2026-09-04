# Release engineering

Voyage publishes `helm` and `vessel` together from one versioned commit. Tags use
`vMAJOR.MINOR.PATCH`. A release is immutable: corrections receive a new tag.

The release workflow builds and tests Linux x86-64, macOS x86-64/ARM64, and Windows
x86-64. Archives contain both binaries, Helm and Vessel manpages, shell completions,
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

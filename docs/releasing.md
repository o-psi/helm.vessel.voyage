# Release engineering

Voyage has no product releases yet. The release workflow packages `helm` and
`vessel` together from one versioned commit. Tags use
`vMAJOR.MINOR.PATCH`. A release is immutable: corrections receive a new tag.

Push and pull-request quality checks run on Linux only to limit development costs.
The tag-triggered release workflow builds archives for Linux x86-64, macOS
x86-64/ARM64, and Windows x86-64; it does not run the workspace test suite on
macOS or Windows. Record separate platform testing before claiming validation.
Archives contain Helm, Vessel and the mock setup-preview executable, Helm and Vessel manpages, shell completions,
the README, linked guides, the example Helm configuration, and a SHA-256 checksum.
Both full-archive packagers use `scripts/release-documents.txt` to include the same
documentation paths without collecting unlisted local notes. Add newly linked
documents to that manifest. Developer build/test commands in the guides require
the source checkout; extracted binaries can run directly from `bin`.
Linux builds can be reproduced locally with:

```sh
cargo build --workspace --release --locked
./scripts/package-release UNIQUE_VERSION
(cd dist && sha256sum -c *.sha256)
python3 tests/system/release_documentation.py
```

Choose a new version label for every packaging attempt. Existing archives and
checksums are never replaced. Packaging failures remove their private staging
directory and leave existing artifacts intact. The documentation fixture extracts
a disposable archive, checks relative links and configuration, runs its binaries,
and verifies missing-guide and repeated-label failures. Native Windows packaging
and clean-install checks remain separate platform evidence.

Before tagging, update versions and changelog, run the full CI command set, complete
the live evaluation suite, inspect generated manpages/completions, and test a clean
archive install. After tagging, compare artifact checksums, smoke-test both binaries
from each archive, attach evaluation/cutover evidence, and publish known limitations.

The workflow uploads build artifacts but intentionally does not auto-publish or sign
a release. Add repository-specific signing and GitHub release credentials only after
the project establishes its key custody and release-approval policy.

Unix builds also package a separate compressed `voyage-installer` with
`scripts/package-installer`. See [setup preview](installer-preview.md) for asset
names and local usage. The small `install.sh` launcher requires these files to be
published as release assets before remote download works; it does not download the
full Voyage archive. Keep the wizard explicitly marked as mocked until real
provisioning is implemented and verified.

## Product capability claims

Release documentation must describe the actual [remote-session surface](remote-sessions.md)
and the remaining [multi-Helm voyage work](voyages.md) separately. A working dedicated
worker does not prove the Helm operator interface, multi-Helm coordination, service
lifecycle or coordinator handoff. Report actual checks and unresolved acceptance
criteria; do not advertise remote dogfood readiness from local tests alone.

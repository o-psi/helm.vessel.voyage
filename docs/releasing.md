# Release engineering

Voyage has no product releases yet. The release workflow packages `helm` and
`vessel` together from one versioned commit. Tags use
`vMAJOR.MINOR.PATCH`. A release is immutable: corrections receive a new tag.

Run `./scripts/check-quality` locally before publication and retain its exact
revision and results; see [quality validation](quality.md). Hosted quality is
manual-only and uses the same Linux suite. Duplicate hosted green checks are not
required after successful local validation.
The tag-triggered release workflow builds archives for Linux x86-64, macOS
x86-64/ARM64, and Windows x86-64; it does not run the workspace test suite on
macOS or Windows. Record separate platform testing before claiming validation.
Archives contain Helm, Vessel and the mock setup-preview executable, Helm and Vessel manpages, shell completions,
the README, linked guides, the example Helm configuration, and a SHA-256 checksum.
Both full-archive packagers use `scripts/release-documents.txt` to include the same
documentation paths without collecting unlisted local notes. Add newly linked
documents and any distributable license files to that manifest. Unlisted files,
including files named `LICENSE*`, are not collected. Developer build/test commands
in the guides require the source checkout; extracted binaries can run directly from `bin`.
Linux builds can be reproduced locally with:

```sh
cargo build --workspace --release --locked
./scripts/package-release UNIQUE_VERSION
(cd dist && sha256sum -c *.sha256)
python3 tests/system/release_documentation.py
```

Choose a new version label for every packaging attempt. Existing archives and
checksums are never replaced. Handled packaging failures remove their private staging
directory and leave existing artifacts intact. The documentation fixture extracts
a disposable archive, checks relative links and configuration, runs its binaries,
and verifies missing-guide and repeated-label failures. The tag workflow checks
guide paths, source bytes and checksums in each platform's archive. Native Windows
execution and clean-install checks remain separate platform evidence.

If a packaging process is forcibly killed, its `.package-*` directory or
`.voyage-*.lock` may remain in `dist`. Confirm that attempt has stopped before
removing its staging directory and lock; preserve published archives and checksums.

Before tagging, update versions and changelog, run `./scripts/check-quality`, complete
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

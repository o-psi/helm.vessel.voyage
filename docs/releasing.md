# Packaging and releases

Voyage is developing its first release. The current packagers distribute `helm`,
`vessel`, the independent `voyage` runtime, and `voyage-installer`. The supported
Linux architecture cutover is implemented; platform/deployment limits are recorded in the
[implementation ledger](implementation.md).

## Build and package

Use a unique label for each packaging attempt:

```sh
cargo build --workspace --release --locked
./scripts/package-release UNIQUE_VERSION
./scripts/package-installer UNIQUE_VERSION
(cd dist && sha256sum -c *.sha256)
```

The Unix full packager requires Python 3.11 or later for release metadata and uses
`target/release`, or `target/TARGET/release` when the
`TARGET` environment variable explicitly selects a platform build. The Windows
packager is `scripts/package-release.ps1`; inspect its declared parameters before
running it for a platform build. Do not infer native Windows execution from an
archive assembled on Linux.

Full archives contain binaries, generated Helm/Vessel/voyage manpages and shell
completions, the example configuration, runtime prompt, agent instructions and the
maintained guides. `scripts/release-documents.txt` is the explicit document allowlist
for both full packagers. All listed paths must exist; reject unsafe/symlinked paths
and keep relative links within the extracted archive valid. Unlisted local notes
and credentials must never enter the archive.

The full archive includes a versioned `release.json` with its label, platform and
all four executable hashes. The standalone installer package contains only the
installer. It can resolve a published full release with `upgrade`, build GitHub
main with explicit `upgrade --dev`, or use local full-release binaries via
`--bin-dir`. `install.sh`
downloads a complete pinned release and opens its real installation wizard; its
assets must be published first. Local builds can be installed before publication.
See [installation, upgrade and rollback](../installer/README.md). Reboot/logout
persistence and non-Linux deployment still require actual native evidence.

## Artifact integrity

Archive publication uses unique labels and must not overwrite existing archives
or checksums. Retain historical artifacts. Handled failures remove their temporary
staging data; forced termination may leave a `.package-*` directory or publication
lock. Verify the originating process stopped before handling those leftovers.

The [quality runner](quality.md) performs packaging and checksum checks. Automated
archive regression fixtures are absent. For documentation changes, inspect archive
membership, guide links, configuration and generated CLI documents explicitly.
Do not claim native runtime or clean-install validation from checksum success.

## Release workflow

Run the applicable clean-checkout quality gates and retain exact evidence before
publication. The tag-driven GitHub and Forgejo workflows build Linux x86-64, macOS
x86-64/ARM64 and Windows x86-64 archives. They upload artifacts; they do not
implicitly sign or publish a product release. Normal quality runs create no tags.

Product release tags use `vMAJOR.MINOR.PATCH` and are immutable. Before a release,
check versioned code, executable help, archive installation behavior and platform
support claims. Record actual runtime/deployment evidence and unresolved limitations.
Do not advertise the target Helm–Vessel–voyage separation, remote multiplexing or
participant execution until the relevant workflows are implemented and verified.

## Independent release signatures and source inventory

The new `packaging/release_bundle.py` supports explicit-asset bundle preparation,
OpenSSH detached signatures and verification against independently pinned trust,
version and target. See the [release bundle guide](https://github.com/o-psi/voyage/blob/main/packaging/README.md).
It copies only selected assets, records source commit/tree and hashes, and emits
an SPDX Cargo.lock inventory. It never restores or implicitly depends on deleted
packaging scripts. Historical packager commands above require those files to be
present; documentation is not evidence that a dirty checkout can run them.

The inventory is not a resolved binary/build SBOM; it includes optional, development
and other-platform dependencies and excludes OS/toolchain inputs. Signatures cover
the manifest and exact inventory/artifact bytes, not proof of their build origin.
Bootstrap and installer acquisition do not yet enforce these signatures. Consumers
must explicitly verify before extracting or executing. No production trust key,
product release, container image, reproducible-build result or native deployment
is implied. Publication of signed assets/full build SBOMs and service deployment
remain explicit delivery work; credentials must never enter source or archives.

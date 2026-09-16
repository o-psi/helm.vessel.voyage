# Packaging and releases

The first binary release is v1.0.0 for Linux x86-64 only. See the
[release guide](releases-v1.0.0.md) for scope and installation. The current packagers distribute `helm`,
`vessel`, the independent `voyage` runtime, and `voyage-installer`. The supported
Linux architecture cutover is implemented; platform/deployment limits are recorded in the
[implementation ledger](implementation.md).

## Build and package

The maintained Linux release packager is `packaging/package_linux.py`.
See [Linux release packaging](../packaging/linux-release.md) for its exact CLI
and failure handling. Build from a clean committed source checkout:

```sh
cargo build --workspace --release --locked -j 8
python3 packaging/package_linux.py --version v1.0.0 --bin-dir target/release --output dist/linux-release
(cd dist/linux-release && sha256sum -c *.sha256)
```

Use a new output directory for every attempt. The packager validates binary versions,
generates CLI documentation, and packages only explicitly selected public documents.
Full archives carry `release.json` with the release identity, target and executable
hashes. The standalone installer contains only the installer, not runtime binaries.
Legacy commands under `scripts/` are not required by this workflow; their presence
in historical commits does not justify restoring deleted files in a working tree.

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
publication. The tag-driven GitHub and Forgejo workflows build Linux x86-64 archives only. They upload artifacts; they do not
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

# Packaging and releases

Voyage is developing its first release. The current packagers distribute `helm`,
`vessel` and the `voyage-installer` setup preview. They do not include a standalone
`voyage` runtime executable because it does not yet exist in this source tree.
Adding it is part of the [implementation work](implementation.md).

## Build and package

Use a unique label for each packaging attempt:

```sh
cargo build --workspace --release --locked
./scripts/package-release UNIQUE_VERSION
./scripts/package-installer UNIQUE_VERSION
(cd dist && sha256sum -c *.sha256)
```

The Unix full packager uses `target/release`, or `target/TARGET/release` when the
`TARGET` environment variable explicitly selects a platform build. The Windows
packager is `scripts/package-release.ps1`; inspect its declared parameters before
running it for a platform build. Do not infer native Windows execution from an
archive assembled on Linux.

Full archives contain binaries, generated Helm/Vessel manpages and shell
completions, the example configuration, runtime prompt, agent instructions and the
maintained guides. `scripts/release-documents.txt` is the explicit document allowlist
for both full packagers. All listed paths must exist; reject unsafe/symlinked paths
and keep relative links within the extracted archive valid. Unlisted local notes
and credentials must never enter the archive.

The standalone installer package contains the setup-preview program. `install.sh`
requires its assets to be published before download works. The wizard's provisioning
actions are mocked; publishing an archive does not make it a working service installer.

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

Signing keys, SBOM publication and service deployment remain explicit delivery work;
credentials for them must not be placed in repository configuration or archives.

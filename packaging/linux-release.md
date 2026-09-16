# Linux x86-64 release packaging

`package_linux.py` is the maintained packager for the Linux x86-64 scope of
[#305](https://github.com/o-psi/voyage/issues/305). It packages already-built,
trusted executables; it does not build, install, tag, sign, upload or publish a
GitHub release. Version changes, builds, workflows and release notes belong to
the release coordinator. No Rust build is needed to test this Python code.

## Prepare assets

Run on Linux x86-64 with Python 3.11 or later, from the checkout root:

```sh
python3 packaging/package_linux.py --version v1.0.0 --bin-dir target/release --output dist/linux-release
(cd dist/linux-release && sha256sum -c *.sha256)
```

All arguments are required. `--bin-dir` explicitly selects the directory containing
`helm`, `vessel`, `voyage` and `voyage-installer`; no PATH lookup, implicit build
output or `TARGET` environment variable is used. Other files in that directory
are ignored. These must be executable regular files, not symlinks. Each copied
executable must have an ELF64 little-endian AMD64 header and report exactly
`NAME 1.0.0` from `--version` for tag `v1.0.0`. Product tags use
`vMAJOR.MINOR.PATCH`, without prerelease or build suffixes. The fixed target is
`x86_64-unknown-linux-gnu`. The ELF check rejects obvious architecture mismatches;
it does not establish glibc compatibility, build provenance or native deployment.
Use trusted builds: version/help generation executes the supplied binaries.

The output directory receives exactly these assets (other existing files remain):

- `voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz`
- `voyage-v1.0.0-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `voyage-installer-x86_64-unknown-linux-gnu.gz`
- `voyage-installer-x86_64-unknown-linux-gnu.gz.sha256`

Checksums use standard `sha256sum` format with the asset basename. The full tarball
has one `voyage-v1.0.0-x86_64-unknown-linux-gnu/` root, four `bin/` executables and
`release.json` schema version 1: `version`, `target` and `binaries` keyed by the
four executable names, each containing `sha256`. This matches the consumers in
[`install.sh`](../install.sh) and the [installer](../installer/README.md).
The standalone gzip decompresses directly to the same installer bytes, not a tar
archive. It still needs the full release for installation.

The three application binaries generate their own manpages and bash, zsh and fish
completions under `share/`. Commands have a 30-second deadline, null stdin,
4 MiB stdout and 64 KiB stderr limits each. The owned process group is killed and
the direct child reaped on completion or failure; diagnostics from those commands
are not echoed. These controls are not an OS sandbox against malicious binaries.

## Document and archive boundaries

[`linux-documents.txt`](linux-documents.txt) is the explicit safe copy allowlist.
Every entry must exist; missing files, traversal, duplicate entries and symlinked
files/ancestors fail packaging. It includes the configuration example, runtime
prompt, agent instructions, selected maintained guides and
[release notes](../docs/releases-v1.0.0.md). It never recursively copies the
checkout, follows documentation links to copy more files, or includes arbitrary
local notes, credentials, build outputs or source files.

For Markdown inline links/images and reference definitions, links to included
files remain relative. Links to unshipped repository files become absolute
`https://github.com/o-psi/voyage/blob/v1.0.0/...` links (using the selected version),
retaining fragments; external URLs and local anchors stay unchanged. This keeps
relative file links valid in the archive without copying unrelated files. It does
not certify external URL availability or heading anchors, and the matching tag
must be published by the coordinator. Plain code examples are not path-validated.
When adding a different Markdown link syntax, extend the transformer and fixtures.
Source files themselves are never rewritten.

Archive owner IDs/names, timestamps and gzip timestamps are normalized. Files have
0644 modes, executables/directories 0755. Bootstrap extraction count and expanded
size limits are enforced. This is not a claim of reproducible binary builds.

## Publication and failure handling

Use a fresh output directory for each release: the standalone asset name is not
versioned. An exclusive `.package-linux.lock` serializes this packager within an
output directory. Existing assets, checksums, dangling symlinks or locks are never
replaced. All four assets are prepared in an owned temporary directory on the
output filesystem, then published with exclusive hard links. Readers must wait
for successful completion: this is no-clobber per-file publication, not an atomic
four-file transaction or crash-durable remote publication protocol.

Handled failures and termination signals remove owned staging and the owned lock.
A partial publication is rolled back only while the destination still has the
published inode identity. Unrelated/concurrently replaced files are preserved.
SIGKILL, power failure or filesystem cleanup errors can leave staging, a lock or
partial assets: verify the originating process has stopped and inspect ownership
before manual recovery. Never blindly remove an existing lock to retry.

## Focused verification

```sh
python3 -m unittest discover -s packaging -p 'test_package_linux.py' -v
python3 packaging/package_linux.py --help
```

Fixtures check archive membership, manifests/checksums, generated document paths,
link rewriting, version rejection, path safety, subprocess bounds, no-clobber and
owned rollback. They mock executable behavior for archive fixtures and separately
exercise real bounded subprocess execution. They do not build Rust or establish
real-binary release, installer, provider or native deployment success. The final
coordinator must run the packager against all four actual version-matched builds,
check checksums and retain installation evidence separately.

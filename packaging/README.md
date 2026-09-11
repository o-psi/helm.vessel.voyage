# Independently verifiable release bundles

`release_bundle.py` is release-maintainer tooling, not an installer or a replacement
for the Linux installation engine. It requires Python 3.11+, Git and OpenSSH with
`ssh-keygen -Y` support. It makes no network requests or provider calls. The current
bootstrap and installer still use HTTPS checksums; they **do not automatically
require these signatures**. Verify before extracting or executing a downloaded
bundle. Use a trusted copy of this verifier, not code from the unverified download.

## Prepare exact assets

Start with a clean committed source checkout, including no untracked files. Build
and check the intended binaries and archives separately. Supply **only** the exact
public assets to publish; no directory traversal or wildcard collection is done.
The utility copies selected files, not their parent directory. Never select private
logs, state, credentials or source archives containing local material.

```sh
python3 packaging/release_bundle.py prepare \
  --source /absolute/clean/checkout --version v0.1.0 --target linux-x86_64 \
  --artifact /absolute/assets/voyage-v0.1.0-linux-x86_64.tar.gz \
  --artifact /absolute/assets/voyage-v0.1.0-linux-x86_64.tar.gz.sha256 \
  --artifact /absolute/assets/voyage-installer-v0.1.0-linux-x86_64.gz \
  --output /absolute/new-bundle
```

These are example paths/version labels, not evidence of published releases. Keep
the output outside the source checkout (or in an already ignored artifact directory).
The output directory must not exist, even if empty or a dangling symlink. Artifact
basenames must be unique. Inputs and their ancestors must not be symlinks. Existing
assets are read and copied, never modified. The source must remain unchanged during
preparation. Run in an owned directory without concurrent writers.

A completed unsigned bundle contains:

- Every explicitly selected asset, byte-for-byte.
- `cargo-lock.spdx.json`: SPDX 2.3 **source dependency inventory**, with each locked
  package's name/version, available Cargo checksum and crates.io package URL.
- `bundle.json`: schema version, release version/target, Git commit/tree/time,
  Cargo.lock hash, and names, sizes and SHA-256 hashes of all selected assets and
  the inventory. This file is written last as the preparation completion marker.

The inventory includes every Cargo.lock entry, even dependencies not compiled for
this target or feature set. It does not claim resolved dependency edges, license
analysis, OS packages, toolchains, generated code or non-Cargo inputs. It is **not a
binary/build SBOM**. The source association is the publisher's declaration; copying
an arbitrary executable beside this inventory cannot prove it was built from that
source. Commit time makes the generated metadata deterministic; identical metadata
is not evidence of reproducible executable builds.

## Sign and independently verify

Select an existing release key, or an SSH public key whose private key is already
available in the local SSH agent. Do not generate a new trust root just to make a
release pass. Do not place signing keys in the bundle, source tree, command text,
logs or CI configuration. Key paths are allowed; passphrases are not CLI inputs.
Encrypted keys should be unlocked in a private terminal/agent beforehand. The
signing subprocess has a 60-second deadline and no interactive standard input.

```sh
python3 packaging/release_bundle.py sign /absolute/new-bundle \
  --key /absolute/private/release-key.pub
```

This signs the exact manifest using the domain-separated OpenSSH namespace
`helm-vessel-voyage-release` and creates `bundle.json.sig` without replacing any
existing signature. Failure leaves the unsigned bundle for inspection. The key
stays on the signing machine. A forced interruption can leave an unsigned bundle
or `.signature-*` staging file; establish that its writer has stopped before
removing only that temporary evidence. Do not publish incomplete bundles.

The consumer's independently provisioned allowed-signers file contains a trusted
public key, for example:

```text
voyage-release namespaces="helm-vessel-voyage-release" ssh-ed25519 PUBLIC_KEY_BASE64
```

The line above is a template, not a usable key. Obtain and pin the real key through
an authenticated channel independent of the downloaded assets. Never trust a key
just because it is next to the release. The verifier refuses trust files inside
the bundle. Key rotation/revocation requires updating this external trust policy;
a signature alone has no expiry or online revocation guarantee.

```sh
python3 packaging/release_bundle.py verify /absolute/downloaded-bundle \
  --allowed-signers /absolute/trusted/release-signers \
  --version v0.1.0 --target linux-x86_64
```

Verification requires the expected version **and** target to prevent accepting a
validly signed wrong release. It first verifies the signature and then every
listed file's name, size and hash. Missing, extra, symlinked or modified files and
unsupported schemas fail. Keep the directory private and unchanged until use;
verification is a point-in-time check, not a filesystem isolation mechanism.

Publish all bundle files together, retaining their basenames, with no-clobber
release upload. Do not publish the external trust file or private signing material.
Signing is not publication: immutable tags/assets, clean-environment hosted
bootstrap/default/pinned installation, native deployment and a production trust
root still need actual evidence. No product release, production signing key or
native-platform support is established by the offline fixture verification.

## Failures and limits

Preparation reserves a fresh output directory and removes only its own new output
on handled failure. An existing destination is never cleaned. Signing uses a
private temporary document and an exclusive hard-link publication of the signature;
repeated signing refuses rather than replacing the first signature. Verification
has no extraction or execution side effects. Subprocess diagnostics are not
forwarded; failures exit nonzero without printing key details. Check the requested
inputs and run ordinary local Git/OpenSSH diagnostics privately if necessary.

No automatic installer trust enforcement, container image, native service, backup
restore or reproducible-build certification is supplied by this utility. Those
remain separate acceptance under [issue #18](https://github.com/o-psi/helm.vessel.voyage/issues/18).

## Recorded Linux verification (issue #18)

Source implementation commit `0e0b03b` was checked on Linux x86-64 with Python
3.14.7 and OpenSSH 10.5p1. Temporary offline fixtures verified deterministic
metadata/inventory, exact copied bytes, existing-output preservation, real SSH
signing and signature no-clobber, independent signer verification, wrong
signer/version/target rejection, unsigned bundles, modified/extra/missing/symlinked
assets, changed manifests, corrupt signatures, input/output ancestor symlinks,
dangling destinations, duplicate names and dirty source refusal. The final
original bundle still verified after the adverse cases.

A separate run prepared a Git source archive of that commit, generated the actual
473-package workspace inventory, signed it with an ephemeral **test** key and
verified every byte. Both fixture and workspace inventories passed the upstream
SPDX 2.3 JSON schema using jsonschema 4.25.1 (schema SHA-256
`239208b7ac287b3cf5d9a9af23f9d69863971102a5e1587a27a398b43490b89b`).
An earlier schema check rejected the document-level `documentComment` field; it
was corrected to SPDX's `comment`, then both schema checks passed. Test private
keys were removed after use; no production signing or trust policy was configured.

Python syntax, command help, relative documentation paths and diff checks passed.
No Rust source, Cargo manifest/lockfile or repository tests changed, so the Rust
coverage measurement was not refreshed. Existing release archives and deleted
root scripts/tests were not changed or restored. The actual source-archive check
is **not** an executable release, binary SBOM or reproducible-build result.

Remaining #18 limitations are concrete: no trusted production release signer was
supplied; no product release/assets or hosted bootstrap/default/pinned install was
published/exercised; no native macOS/Windows execution target was supplied; and
Docker daemon access on this Linux account failed with socket permission denied.
No container execution is claimed. Reboot/logout validation requires a disposable
or separately scheduled deployment, not rebooting the host running concurrent
voyages. Backup/restore must preserve current grant revocation and key custody;
this utility neither copies private state nor establishes that recovery contract.
The broad issue remains open, without a human acceptance gate.

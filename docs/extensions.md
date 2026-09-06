# Declarative extension packages

Helm can install UTF-8 skills and resources without rebuilding the program. A
package is inert until you enable its exact archive digest. Enabled skill text is
untrusted model context: it cannot grant tools, change configured roots, approve
commands or override the selected sandbox. Executable extension entrypoints and
SDK tools remain planned under [#63](https://github.com/o-psi/voyage/issues/63) and
[#75](https://github.com/o-psi/voyage/issues/75). Package operations run no install
scripts, commands or MCP servers. Lifecycle commands are explicit administrative
CLI actions, handled before provider configuration; they are not agent filesystem
tools and do not prompt through tool-access approvals. Enabling authorizes only the
exact package's `model_context`, never execution or a broader access mode.

## Build and inspect a package

Create a directory containing `manifest.json` and the declared files. For example,
`skill.md` might contain instructions for reviewing prose. Compute its SHA-256 from
its exact UTF-8 bytes and insert it into this manifest:

```json
{
  "format": 1,
  "id": "prose-review",
  "version": "1.0.0",
  "helm": "0.1",
  "capabilities": ["model_context"],
  "contents": [
    {"path": "skill.md", "kind": "skill", "sha256": "REPLACE_WITH_64_LOWERCASE_HEX_DIGITS"}
  ],
  "entrypoints": ["skill.md"]
}
```

`helm` selects an exact supported major/minor compatibility line. Version numbers
use numeric `major.minor.patch`. Content kinds are `skill` or `resource`; entrypoints
name declared skills. Resources remain inert and can be inspected explicitly. The
only supported requested capability is `model_context`. Unknown fields, unsupported
capabilities, duplicate keys, undeclared files and mismatched hashes are rejected.

```sh
helm extension pack ./prose-review ./prose-review.helmpkg
helm extension install ./prose-review.helmpkg
helm extension list
helm extension inspect prose-review
helm extension resource prose-review skill.md
```

You can also install directly from the source directory. `pack` creates a new
output file and refuses to overwrite one. A `.helmpkg` is strict JSON containing
`manifest` and a `files` object mapping each declared path to its exact text. It has
no compression or extraction step. Archives are limited to 64 KiB and 32 files.
Portable paths use lowercase ASCII letters, digits, hyphens, underscores, dots and
slash separators, with no dot components, leading/trailing dots, Windows reserved
names, symlinks or special files. Package versions and identities are data, never
command names.

## Enable, update and remove

Review the manifest and resource text before enabling. Substitute the archive
`digest` printed by inspection in these commands:

```sh
helm extension enable prose-review --expected DIGEST
helm extension update prose-review ./new.helmpkg --expected OLD_DIGEST
helm extension inspect prose-review
helm extension enable prose-review --expected NEW_DIGEST
helm extension disable prose-review --expected NEW_DIGEST
helm extension remove prose-review --expected NEW_DIGEST
```

Installation and update are inactive. Updating first durably clears the previous
activation; a publication failure may leave the old package installed but inactive.
A stale expected digest fails without replacing installed bytes. After any uncertain
write, inspect the current state before retrying. Disable and removal remain
available when activation capacity is full. Grants describe current state and do
not consume permanent lifetime slots. `helm extension grants` lists current binding
IDs and digests. If a workspace or package was removed outside Helm, revoke its
orphaned binding with `helm extension revoke-grant BINDING --expected DIGEST`;
this does not require recreating the original package or workspace.

Packages are stored in a bounded `catalog.json`: user packages live under Helm's
platform data directory at `extensions/catalog.json`; project packages live at
`WORKSPACE/.helm/extensions/catalog.json`. Pass `--scope project` to an extension
command to operate on the project catalog. Project packages shadow same-identity
user packages even when inactive or individually invalid. Removing a project
candidate reveals the user candidate again. A structurally corrupt project catalog
suppresses package guidance, rather than guessing which user packages to activate.
Each catalog allows 128 packages and 1 MiB of encoded JSON.

Activation records live separately in private user data at
`extension-grants/grants.json`. They bind scope, workspace, identity and exact
archive bytes; copying a project catalog does not copy consent. Paths must be
UTF-8. Corrupt or missing activation records cannot activate packages. Preserve a
corrupt file before explicit operator repair; Helm does not silently replace it.
The private storage lock serializes package mutations and complete runtime snapshots.
A busy or unavailable catalog/grant store omits package guidance with a diagnostic,
while ordinary chat remains available. Inspect lists per-scope activation; a valid
active user candidate can still be shadowed by a project candidate.

## HTTPS indexes

An index configuration is a local JSON file with `format: 1`, an HTTPS `url`, and
its exact index-byte `sha256`. An optional `ca_certificate` names a PEM trust root
for a private index; relative paths resolve beside the configuration file. The CA
is an explicit operator choice, never supplied by downloaded packages.

The index document has `format: 1` and a `packages` array. Each entry contains `id`,
an absolute HTTPS archive `url`, and the archive-byte `sha256`. Every archive URL
must have the same origin as the index. Package identities are unique within an
index. URLs cannot contain credentials, queries or fragments.

```sh
helm extension fetch ./trusted-index.json prose-review
helm extension fetch ./trusted-index.json prose-review --expected OLD_DIGEST
```

The second form updates exact installed bytes and clears activation. Helm verifies
TLS and both digests, refuses redirects and downgrade, sends no credentials, and
disables proxy inheritance. Requests have a 5-second connection and 20-second total
request deadline. Index and archive responses are each limited to 64 KiB; indexes
allow 128 entries. HTTP bodies and secrets are not included in errors. A hash checks
bytes, not publisher identity or the safety of instructions. Runtime discovery
never contacts an index.

## Runtime and recovery

The shared agent reads and validates one complete activated-package snapshot at the
start of each run. That snapshot remains fixed through provider/tool follow-ups;
install, update, disable and remove affect subsequent runs. Guidance is redacted
before transport, followed by Helm's authoritative runtime guidance, and excluded
from canonical session history. It still consumes the provider's context budget.
The aggregate package-guidance limit is 64 KiB; exceeding it omits package guidance
with a diagnostic. This does not prevent normal chat startup.

Private files protect against other OS users; they do not isolate code already
running as the same OS account. Package text remains subject to ordinary model
uncertainty and prompt injection. Local command policy, approvals and an enabled
OS sandbox remain the execution boundary. The package system does not add a new
execution path or imply trust in a package author.

Deterministic tests include archive/path/integrity validation, lifecycle and CAS,
project shadowing, corrupt grants, bounded capacity, failed publication, native
provider snapshots and redaction, and local TLS index fixtures. Linux execution
results are recorded with delivery evidence. Native macOS and Windows checks remain
separate from Linux coverage.

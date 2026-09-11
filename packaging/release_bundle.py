#!/usr/bin/env python3
"""Prepare and verify explicitly selected release assets; Python 3.11 + OpenSSH."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

NAMESPACE = "helm-vessel-voyage-release"
PRINCIPAL = "voyage-release"
MANIFEST = "bundle.json"
SBOM = "cargo-lock.spdx.json"
RESERVED = {MANIFEST, MANIFEST + ".sig", SBOM}
LIMIT = 1024 * 1024


class Refusal(ValueError):
    """An authored diagnostic safe to display without subprocess/private data."""


def regular(path):
    """No symlink components, devices, pipes, or directories as inputs."""
    path = Path(os.path.abspath(path))
    for part in [path, *path.parents]:
        if part.is_symlink():
            raise Refusal("symlink input or ancestor refused")
    if not stat.S_ISREG(path.stat().st_mode):
        raise Refusal("input must be a regular file")
    return path


def digest(path):
    with regular(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def command(args, **kwargs):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=60, check=False, **kwargs)
    if result.returncode:
        raise Refusal("external command failed: " + args[0])
    return result.stdout


def source_identity(source):
    def git(*args):
        return command(["git", "-C", str(source), *args]).decode().strip()
    if Path(git("rev-parse", "--show-toplevel")) != source:
        raise Refusal("source must be the checkout root")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise Refusal("source checkout must be clean, including untracked files")
    commit = git("rev-parse", "HEAD")
    tree = git("rev-parse", "HEAD^{tree}")
    epoch = int(git("show", "-s", "--format=%ct", "HEAD"))
    lock_digest = digest(source / "Cargo.lock")
    committed_lock = command(["git", "-C", str(source), "show", "HEAD:Cargo.lock"])
    if hashlib.sha256(committed_lock).hexdigest() != lock_digest:
        raise Refusal("Cargo.lock must match the committed source")
    return {"commit": commit, "tree": tree, "commit_time": epoch,
            "cargo_lock_sha256": lock_digest}


def lock_inventory(source, identity, version):
    raw = regular(source / "Cargo.lock").read_bytes()
    packages = tomllib.loads(raw.decode())["package"]
    entries = []
    for i, package in enumerate(packages):
        origin = package.get("source", "")
        entry = {
            "SPDXID": f"SPDXRef-Package-{i}", "name": package["name"],
            "versionInfo": package["version"], "downloadLocation": "NOASSERTION",
            "filesAnalyzed": False, "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION", "copyrightText": "NOASSERTION",
            "comment": ("Workspace/path dependency" if not origin else
                        "Cargo registry dependency" if origin.startswith("registry+") else
                        "Non-registry source pinned in Cargo.lock"),
        }
        if origin == "registry+https://github.com/rust-lang/crates.io-index":
            entry["downloadLocation"] = (
                f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download")
            entry["externalRefs"] = [{"referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": f"pkg:cargo/{package['name']}@{package['version']}"}]
        if "checksum" in package:
            entry["checksums"] = [{"algorithm": "SHA256", "checksumValue": package["checksum"]}]
        entries.append(entry)
    created = datetime.datetime.fromtimestamp(identity["commit_time"], datetime.timezone.utc)
    namespace_hash = hashlib.sha256(encoded({"source": identity, "version": version})).hexdigest()
    return {
        "spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"Voyage {version} Cargo.lock inventory",
        "documentNamespace": "https://spdx.org/spdxdocs/voyage-lock-" + namespace_hash,
        "creationInfo": {"created": created.strftime("%Y-%m-%dT%H:%M:%SZ"),
                         "creators": ["Tool: voyage-release-bundle-1"]},
        "comment": (
            "Source dependency inventory, NOT a resolved binary/build SBOM. Includes all lockfile "
            "platforms and optional/dev dependencies; excludes OS packages, toolchains, generated "
            "code and non-Cargo inputs. No license audit or dependency-edge claim. Creation time "
            "is the source commit time for deterministic output. Artifact/source association is "
            "a publisher declaration, not build provenance or reproducibility evidence."),
        "packages": entries,
        "relationships": [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES",
                           "relatedSpdxElement": p["SPDXID"]} for p in entries],
    }


def safe_name(name):
    return bool(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,199}", name))


def prepare(args):
    source = Path(args.source).absolute()
    identity = source_identity(source)
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", args.version):
        raise Refusal("version must be vMAJOR.MINOR.PATCH or a prerelease")
    if not safe_name(args.target):
        raise Refusal("invalid target label")
    if not 1 <= len(args.artifact) <= 64:
        raise Refusal("select between 1 and 64 artifacts explicitly")
    inputs = [regular(p) for p in args.artifact]
    names = [p.name for p in inputs]
    if len(set(names)) != len(names) or any(not safe_name(n) or n in RESERVED for n in names):
        raise Refusal("invalid, duplicate or reserved artifact basename")
    output = Path(os.path.abspath(args.output))
    # Reserve the exact destination; never replace a directory or dangling symlink.
    for parent in output.parents:
        if parent.is_symlink():
            raise Refusal("symlink output ancestor refused")
    output.mkdir(mode=0o700)
    try:
        files = {}
        for item in inputs:
            destination = output / item.name
            before = digest(item)
            with item.open("rb") as src, destination.open("xb") as dst:
                shutil.copyfileobj(src, dst)
            if digest(destination) != before or digest(item) != before:
                raise Refusal("artifact changed during copy")
            files[item.name] = {"sha256": before, "bytes": destination.stat().st_size}
        (output / SBOM).write_bytes(encoded(lock_inventory(source, identity, args.version)))
        files[SBOM] = {"sha256": digest(output / SBOM), "bytes": (output / SBOM).stat().st_size}
        if source_identity(source) != identity:
            raise Refusal("source changed during preparation")
        manifest = {"schema_version": 1, "version": args.version, "target": args.target,
                    "source": identity, "files": files,
                    "association": "Publisher-declared source; not verified build provenance"}
        # Last file is the completion marker. An interrupted preparation is unsigned.
        (output / MANIFEST).write_bytes(encoded(manifest))
    except BaseException:
        shutil.rmtree(output)
        raise
    print("Prepared unsigned bundle; independently trusted signature still required")


def check_contents(bundle):
    manifest = regular(bundle / MANIFEST)
    if manifest.stat().st_size > LIMIT:
        raise Refusal("manifest exceeds limit")
    data = json.loads(manifest.read_bytes())
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise Refusal("unsupported bundle schema")
    files = data.get("files")
    if not isinstance(files, dict) or not 2 <= len(files) <= 65 or SBOM not in files:
        raise Refusal("invalid artifact inventory")
    expected = set(files) | {MANIFEST}
    if (bundle / (MANIFEST + ".sig")).exists():
        expected.add(MANIFEST + ".sig")
    if set(p.name for p in bundle.iterdir()) != expected:
        raise Refusal("unlisted or missing bundle files")
    for name, record in files.items():
        if not safe_name(name) or name in {MANIFEST, MANIFEST + ".sig"}:
            raise Refusal("invalid artifact name")
        if not isinstance(record, dict):
            raise Refusal("invalid artifact record")
        path = regular(bundle / name)
        if path.stat().st_size != record["bytes"] or digest(path) != record["sha256"]:
            raise Refusal("artifact integrity mismatch")
    return data


def sign(args):
    bundle = Path(args.bundle).absolute()
    check_contents(bundle)
    signature = bundle / (MANIFEST + ".sig")
    if os.path.lexists(signature):
        raise Refusal("signature already exists; no replacement permitted")
    key = regular(args.key)
    if key.is_relative_to(bundle):
        raise Refusal("signing key must not be inside bundle")
    # OpenSSH writes a sibling .sig; isolate that effect, then no-clobber link it.
    with tempfile.TemporaryDirectory(prefix="voyage-sign-") as tmp:
        document = Path(tmp) / MANIFEST
        document.write_bytes(regular(bundle / MANIFEST).read_bytes())
        command(["ssh-keygen", "-Y", "sign", "-n", NAMESPACE, "-f", str(key), str(document)],
                stdin=subprocess.DEVNULL, start_new_session=True,
                env={**os.environ, "SSH_ASKPASS_REQUIRE": "never"})
        if document.read_bytes() != regular(bundle / MANIFEST).read_bytes():
            raise Refusal("manifest changed during signing")
        check_contents(bundle)
        # The temporary directory may be on another filesystem: stage next to destination.
        fd, staged = tempfile.mkstemp(prefix=".signature-", dir=bundle.parent)
        try:
            with os.fdopen(fd, "wb") as stream:
                stream.write(document.with_suffix(".json.sig").read_bytes())
            os.link(staged, signature)
        finally:
            os.unlink(staged)
    print("Signed bundle; verify with independently pinned allowed-signers file")


def verify(args):
    bundle = Path(args.bundle).absolute()
    manifest = regular(bundle / MANIFEST)
    signature = regular(bundle / (MANIFEST + ".sig"))
    trust = regular(args.allowed_signers)
    if trust.is_relative_to(bundle):
        raise Refusal("trust must be pinned independently, outside the bundle")
    if manifest.stat().st_size > LIMIT or signature.stat().st_size > LIMIT:
        raise Refusal("signature or manifest exceeds limit")
    signed_bytes = manifest.read_bytes()
    command(["ssh-keygen", "-Y", "verify", "-f", str(trust), "-I", PRINCIPAL,
             "-n", NAMESPACE, "-s", str(signature)], input=signed_bytes)
    data = check_contents(bundle)
    if manifest.read_bytes() != signed_bytes:
        raise Refusal("manifest changed during verification")
    if data["version"] != args.version or data["target"] != args.target:
        raise Refusal("signed bundle is not the requested version/target")
    print("Verified signature and every listed byte for requested version/target; not build provenance")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    create = commands.add_parser("prepare", help="copy explicit assets and generate source inventory")
    create.add_argument("--source", required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--target", required=True)
    create.add_argument("--artifact", action="append", required=True)
    create.add_argument("--output", required=True)
    signer = commands.add_parser("sign", help="sign bundle with existing key or agent-backed public key")
    signer.add_argument("bundle")
    signer.add_argument("--key", required=True)
    checker = commands.add_parser("verify", help="verify pinned signer, version, target and all bytes")
    checker.add_argument("bundle")
    checker.add_argument("--allowed-signers", required=True)
    checker.add_argument("--version", required=True)
    checker.add_argument("--target", required=True)
    args = parser.parse_args()
    try:
        {"prepare": prepare, "sign": sign, "verify": verify}[args.action](args)
    except Refusal as error:
        print(str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired):
        # No raw subprocess diagnostics, local paths or signing details in public output.
        print("Release operation refused or failed; check inputs, integrity and local tool availability", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

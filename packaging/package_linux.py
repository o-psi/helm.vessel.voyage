#!/usr/bin/env python3
"""Package explicitly selected, already-built Linux x86-64 release binaries."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import posixpath
import re
import selectors
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
from urllib.parse import quote, unquote, urlsplit

ROOT = Path(__file__).absolute().parent.parent
TARGET = "x86_64-unknown-linux-gnu"
BINARIES = ("helm", "vessel", "voyage", "voyage-installer")
TIMEOUT = 30
OUTPUT_LIMIT = 4 * 1024 * 1024
# Repository Markdown uses inline links and reference definitions. Do not alter
# plain text/code paths or copy the destination of a link implicitly.
LINK = re.compile(r'(?P<inline>!?\[[^\]\n]*\]\()(?P<url><[^>\n]+>|[^\s)]+)'
                  r'|(?P<reference>^ {0,3}\[[^\]\n]+\]:[ \t]*)(?P<refurl><[^>\n]+>|\S+)', re.M)


def regular(path: Path) -> Path:
    """Reject symlinks in every component, including the explicitly supplied dir."""
    path = path.absolute()
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError(f"symlink is not permitted: {part}")
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError(f"not a regular file: {path}")
    return path


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_elf(path: Path) -> None:
    with path.open("rb") as stream:
        header = stream.read(20)
    # ELF64, little endian, current ELF version, executable/shared-object, AMD64.
    if (len(header) != 20 or header[:7] != b"\x7fELF\x02\x01\x01"
            or struct.unpack_from("<HH", header, 16) not in ((2, 62), (3, 62))):
        raise ValueError(f"not an x86-64 Linux ELF executable: {path.name}")


def run_binary(binary: Path, *args: str, timeout: float = TIMEOUT) -> bytes:
    """Noninteractive, wall-time/output-bounded command; reap the owned group."""
    process = subprocess.Popen([str(binary), *args], stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True)
    output = bytearray()
    sizes = {"stdout": 0, "stderr": 0}
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError(f"command timed out: {binary.name} {' '.join(args)}")
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    sizes[key.data] += len(chunk)
                    if sizes[key.data] > (OUTPUT_LIMIT if key.data == "stdout" else 65536):
                        raise ValueError(f"command output limit exceeded: {binary.name}")
                    if key.data == "stdout":
                        output.extend(chunk)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError(f"command timed out: {binary.name}")
            try:
                code = process.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                raise ValueError(f"command timed out: {binary.name}") from error
            if code:
                # Subprocess diagnostics are deliberately not echoed into logs.
                raise ValueError(f"command failed ({code}): {binary.name} {' '.join(args)}")
        return bytes(output)
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)
        process.stdout.close()
        process.stderr.close()


def document_paths(root: Path) -> list[str]:
    paths = regular(root / "packaging/linux-documents.txt").read_text().splitlines()
    if len(paths) != len(set(paths)):
        raise ValueError("duplicate document allowlist entry")
    for name in paths:
        path = PurePosixPath(name)
        if (not re.fullmatch(r"[A-Za-z0-9_./-]+", name) or path.is_absolute()
                or any(part in ("", ".", "..") for part in name.split("/"))
                or path.parts[0] in ("bin", "share") or name == "release.json"):
            raise ValueError(f"unsafe document allowlist entry: {name}")
        regular(root / name)
    return paths


def release_markdown(text: str, name: str, included: set[str], version: str) -> str:
    """Retain shipped relative links; pin unshipped source links to the release tag."""
    def replace(match: re.Match) -> str:
        original = match.group("url") or match.group("refurl")
        url = original.strip("<>")
        parsed = urlsplit(url)
        if parsed.scheme or parsed.netloc or not parsed.path:
            return match.group(0)
        dest = posixpath.normpath(posixpath.join(posixpath.dirname(name), unquote(parsed.path)))
        if dest.startswith("../") or dest.startswith("/"):
            raise ValueError(f"document link escapes release root: {name}: {url}")
        if dest in included:
            return match.group(0)
        replacement = f"https://github.com/o-psi/voyage/blob/{version}/{quote(dest, safe='/')}"
        if parsed.query:
            replacement += "?" + parsed.query
        if parsed.fragment:
            replacement += "#" + parsed.fragment
        if original.startswith("<"):
            replacement = "<" + replacement + ">"
        return (match.group("inline") or match.group("reference")) + replacement
    return LINK.sub(replace, text)


def write_bytes(root: Path, name: str, data: bytes) -> None:
    if not data.strip():
        raise ValueError(f"empty generated document: {name}")
    destination = root / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    destination.chmod(0o644)


def populate(root: Path, bin_dir: Path, source: Path, version: str) -> None:
    (root / "bin").mkdir(parents=True)
    for name in BINARIES:
        binary = regular(bin_dir / name)
        if not os.access(binary, os.X_OK):
            raise ValueError(f"binary is not executable: {name}")
        copied = root / "bin" / name
        shutil.copyfile(binary, copied)
        copied.chmod(0o755)
        validate_elf(copied)
        reported = run_binary(copied, "--version").decode("utf-8").strip()
        # Clap emits the package version without the product tag's leading v.
        if reported != f"{name} {version[1:]}":
            raise ValueError(f"binary version mismatch: {name}; expected {version[1:]}")
    paths = document_paths(source)
    included = set(paths)
    for name in paths:
        data = (source / name).read_bytes()
        if name.endswith(".md"):
            data = release_markdown(data.decode("utf-8"), name, included, version).encode("utf-8")
        write_bytes(root, name, data)
    for name in BINARIES[:3]:
        binary = root / "bin" / name
        write_bytes(root, f"share/man/man1/{name}.1", run_binary(binary, "manpage"))
        for shell, destination in (
            ("bash", f"share/bash-completion/completions/{name}"),
            ("zsh", f"share/zsh/site-functions/_{name}"),
            ("fish", f"share/fish/vendor_completions.d/{name}.fish"),
        ):
            write_bytes(root, destination, run_binary(binary, "completions", shell))
    manifest = {"schema_version": 1, "version": version, "target": TARGET,
                "binaries": {name: {"sha256": digest(root / "bin" / name)} for name in BINARIES}}
    write_bytes(root, "release.json", (json.dumps(manifest, indent=2) + "\n").encode())


def normalized(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.uid = info.gid = info.mtime = 0
    info.uname = info.gname = ""
    info.mode = 0o755 if info.isdir() or "/bin/" in info.name else 0o644
    return info


def package(bin_dir: Path, version: str, output: Path, *, source: Path = ROOT) -> list[Path]:
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("version must be a product tag such as v1.0.1")
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise ValueError("this packager executes binaries and requires Linux x86-64")
    output = output.absolute()
    bin_dir = bin_dir.absolute()
    output.mkdir(parents=True, exist_ok=True)
    for part in (output, *output.parents):
        if part.is_symlink():
            raise ValueError(f"symlink in output path: {part}")
    basename = f"voyage-{version}-{TARGET}"
    full = basename + ".tar.gz"
    installer = f"voyage-installer-{TARGET}.gz"
    names = [full, full + ".sha256", installer, installer + ".sha256"]
    lock = output / ".package-linux.lock"
    lock.mkdir()  # Never adopt or remove someone else's lock.
    published: list[tuple[Path, os.stat_result]] = []
    try:
        if any(os.path.lexists(output / name) for name in names):
            raise FileExistsError("release assets already exist; use a fresh output directory")
        with tempfile.TemporaryDirectory(prefix=".package-linux-", dir=output) as temporary:
            stage = Path(temporary)
            root = stage / basename
            populate(root, bin_dir, source, version)
            # Bound membership and expanded bytes to install.sh's extraction limits.
            members = sorted(root.rglob("*"))
            if len(members) + 1 > 4096 or sum(p.stat().st_size for p in members if p.is_file()) > 1073741824:
                raise ValueError("release exceeds bootstrap extraction limits")
            with (stage / full).open("xb") as stream:
                with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
                    with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                        archive.add(root, arcname=basename, recursive=False, filter=normalized)
                        for path in members:
                            archive.add(path, arcname=f"{basename}/{path.relative_to(root).as_posix()}",
                                        recursive=False, filter=normalized)
            with (stage / installer).open("xb") as stream:
                with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
                    with (root / "bin/voyage-installer").open("rb") as binary:
                        shutil.copyfileobj(binary, compressed)
            if (stage / full).stat().st_size > 536870912:
                raise ValueError("release exceeds installer download limit")
            for name in (full, installer):
                (stage / (name + ".sha256")).write_text(f"{digest(stage / name)}  {name}\n")
            for name in names:
                staged, destination = stage / name, output / name
                identity = staged.stat()
                os.link(staged, destination)  # Atomic, no replace, including dangling symlinks.
                published.append((destination, identity))
        return [output / name for name in names]
    except BaseException:
        for path, identity in reversed(published):
            try:
                current = path.lstat()
                if (current.st_dev, current.st_ino) == (identity.st_dev, identity.st_ino):
                    path.unlink()
            except FileNotFoundError:
                pass
        raise
    finally:
        lock.rmdir()


def interrupted(number: int, _frame: object) -> None:
    raise InterruptedError(f"packaging interrupted by signal {number}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True, help="directory containing the four prebuilt executables")
    parser.add_argument("--version", required=True, help="product tag, e.g. v1.0.1 (binaries report 1.0.1)")
    parser.add_argument("--output", type=Path, required=True, help="local output directory; assets are never overwritten")
    args = parser.parse_args()
    for number in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(number, interrupted)
    try:
        for path in package(args.bin_dir, args.version, args.output):
            print(path.name)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"packaging failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

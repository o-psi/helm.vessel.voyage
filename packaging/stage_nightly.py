"""Inventory a nightly archive's browser assets for voyage-installer."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat

import browser_assets
import update_compatibility


BINARIES = ("helm", "vessel", "voyage", "voyage-installer")
TARGETS = {
    "x86_64-unknown-linux-gnu": "",
    "x86_64-apple-darwin": "",
    "x86_64-pc-windows-msvc": ".exe",
}
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{8}\.[0-9]+\.[0-9]+\Z")


def stage(root: Path, version: str, source: Path,
          target: str = "x86_64-unknown-linux-gnu") -> None:
    if target not in TARGETS:
        raise ValueError("Unsupported nightly archive target")
    suffix = TARGETS[target]
    if not VERSION.fullmatch(version):
        raise ValueError("Expected a unique nightly version")
    if (root / "release.json").exists():
        raise ValueError("Nightly release manifest already exists")
    binaries = {}
    for name in BINARIES:
        filename = name + suffix
        path = root / "bin" / filename
        mode = path.lstat().st_mode
        # Windows executable files have no portable POSIX execute-bit contract.
        if not stat.S_ISREG(mode) or (not suffix and not mode & 0o111) or path.stat().st_size == 0:
            raise ValueError(f"Invalid nightly executable: {filename}")
        binaries[filename] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    assets = browser_assets.stage(source, root)
    if not assets:
        raise ValueError("Nightly browser distribution is missing")
    manifest = {
        "schema_version": 1,
        "version": version,
        "target": target,
        "binaries": binaries,
        "assets": assets,
        "update_compatibility": update_compatibility.contract(Path(__file__).resolve().parent.parent),
    }
    with (root / "release.json").open("x") as output:
        json.dump(manifest, output, indent=2, sort_keys=True)
        output.write("\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="nightly archive root containing bin/")
    parser.add_argument("version", help="nightly version emitted by prepare_nightly.py")
    parser.add_argument("--target", choices=tuple(TARGETS), default="x86_64-unknown-linux-gnu")
    parser.add_argument("--browser-source", type=Path, default=Path("voyage/browser"))
    args = parser.parse_args()
    stage(args.root, args.version, args.browser_source, args.target)


if __name__ == "__main__":
    main()

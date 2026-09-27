"""Inventory a nightly archive's browser assets for voyage-installer."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import stat

import browser_assets


BINARIES = ("helm", "vessel", "voyage", "voyage-installer")
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{8}\.[0-9]+\.[0-9]+\Z")


def stage(root: Path, version: str, source: Path) -> None:
    if not VERSION.fullmatch(version):
        raise ValueError("Expected a unique nightly version")
    if (root / "release.json").exists():
        raise ValueError("Nightly release manifest already exists")
    binaries = {}
    for name in BINARIES:
        path = root / "bin" / name
        mode = path.lstat().st_mode
        if not stat.S_ISREG(mode) or not mode & 0o111:
            raise ValueError(f"Invalid nightly executable: {name}")
        binaries[name] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    assets = browser_assets.stage(source, root)
    if not assets:
        raise ValueError("Nightly browser distribution is missing")
    manifest = {
        "schema_version": 1,
        "version": version,
        "target": "x86_64-unknown-linux-gnu",
        "binaries": binaries,
        "assets": assets,
    }
    with (root / "release.json").open("x") as output:
        json.dump(manifest, output, indent=2, sort_keys=True)
        output.write("\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="nightly archive root containing bin/")
    parser.add_argument("version", help="nightly version emitted by prepare_nightly.py")
    parser.add_argument("--browser-source", type=Path, default=Path("voyage/browser"))
    args = parser.parse_args()
    stage(args.root, args.version, args.browser_source)


if __name__ == "__main__":
    main()

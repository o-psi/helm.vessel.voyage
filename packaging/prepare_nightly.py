#!/usr/bin/env python3
"""Apply a unique nightly version in an ephemeral GitHub Actions checkout only."""
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
STABLE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Refusing to modify versions outside GitHub Actions")
    target = (ROOT / "packaging/nightly-version.txt").read_text().strip()
    match = STABLE.fullmatch(target)
    if not match:
        raise SystemExit("nightly-version.txt must contain a stable MAJOR.MINOR.PATCH target")
    target_numbers = tuple(map(int, match.groups()))
    tags = subprocess.check_output(["git", "tag", "--list"], cwd=ROOT, text=True)
    for tag in tags.splitlines():
        shipped = STABLE.fullmatch(tag.removeprefix("v")) if tag.startswith("v") else None
        if shipped and tuple(map(int, shipped.groups())) >= target_numbers:
            raise SystemExit(f"Nightly target must exceed shipped stable tag {tag}; update nightly-version.txt")
    run = os.environ["GITHUB_RUN_ID"]
    attempt = os.environ["GITHUB_RUN_ATTEMPT"]
    if not all(re.fullmatch(r"[1-9][0-9]*", value) for value in (run, attempt)):
        raise SystemExit("Invalid GitHub run identity")
    version = f"{target}-nightly.{datetime.now(timezone.utc):%Y%m%d}.{run}.{attempt}"
    manifest_path = ROOT / "Cargo.toml"
    lock_path = ROOT / "Cargo.lock"
    manifest = manifest_path.read_text()
    workspace = tomllib.loads(manifest)["workspace"]
    old = workspace["package"]["version"]
    members = set()
    edits = {}
    for member in workspace["members"]:
        path = ROOT / member / "Cargo.toml"
        text = path.read_text()
        package = tomllib.loads(text)["package"]
        if package["version"] == old:
            text, count = re.subn(r'(?m)^version = "' + re.escape(old) + r'"$',
                                  'version = "' + version + '"', text)
            if count != 1:
                raise SystemExit(f"Cannot uniquely locate package version in {member}")
            edits[path] = text
        elif package["version"] != {"workspace": True}:
            raise SystemExit(f"Unexpected package version in {member}")
        members.add(package["name"])
    manifest, count = re.subn(
        r'(\[workspace\.package\]\n(?:(?!\[)[^\n]*\n)*?version = ")' + re.escape(old) + r'"',
        lambda m: m[1] + version + '"', manifest, count=1,
    )
    if count != 1:
        raise SystemExit("Cannot locate workspace version")
    lock = lock_path.read_text()
    for member in sorted(members):
        lock, count = re.subn(
            r'(\[\[package\]\]\nname = "' + re.escape(member) + r'"\nversion = ")' + re.escape(old) + r'"',
            lambda m: m[1] + version + '"', lock,
        )
        if count != 1:
            raise SystemExit(f"Cannot uniquely locate workspace lock entry for {member}")
    # Validate all edits before writing either file. cargo build --locked then
    # refuses any additional resolution change rather than silently updating it.
    tomllib.loads(manifest)
    tomllib.loads(lock)
    for path, text in edits.items():
        tomllib.loads(text)
    for path, text in edits.items():
        path.write_text(text)
    manifest_path.write_text(manifest)
    lock_path.write_text(lock)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"version={version}\n")


if __name__ == "__main__":
    main()

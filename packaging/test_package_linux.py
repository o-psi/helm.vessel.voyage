"""Focused offline fixtures; no Rust compilation or release publication."""
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_linux as pack


class PackagingTests(unittest.TestCase):
    def setUp(self):
        # Keep all fixture effects within the isolated checkout.
        self.temp = tempfile.TemporaryDirectory(prefix=".fixture-", dir=Path(__file__).parent)
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.source = self.base / "source"
        (self.source / "packaging").mkdir(parents=True)
        (self.source / "packaging/linux-documents.txt").write_text("README.md\ndocs/guide.md\n")
        (self.source / "docs").mkdir()
        (self.source / "README.md").write_text("[Guide](docs/guide.md#intro) [Code](src/main.rs)\n")
        (self.source / "docs/guide.md").write_text("# Intro\n[Home](../README.md)\n")
        (self.source / "secret.txt").write_text("not for release")
        self.bins = self.base / "bins"
        self.bins.mkdir()
        header = b"\x7fELF\x02\x01\x01" + bytes(9) + struct.pack("<HH", 3, 62)
        for name in pack.BINARIES:
            path = self.bins / name
            path.write_bytes(header + name.encode())
            path.chmod(0o755)
        (self.bins / "private").write_text("never ship")
        self.output = self.base / "output"

    @staticmethod
    def command(binary, *args):
        if args == ("--version",):
            return f"{binary.name} 1.0.0\n".encode()
        return f"{binary.name}: {' '.join(args)}\n".encode()

    def package(self):
        with patch.object(pack, "run_binary", side_effect=self.command):
            return pack.package(self.bins, "v1.0.0", self.output, source=self.source)

    def assert_clean(self):
        self.assertEqual(list(self.output.iterdir()), [])

    def test_archive_contract_and_links(self):
        files = self.package()
        self.assertEqual(len(files), 4)
        for path in (files[0], files[2]):
            self.assertEqual(Path(str(path) + ".sha256").read_text(),
                             f"{pack.digest(path)}  {path.name}\n")
        self.assertEqual(gzip.decompress(files[2].read_bytes()),
                         (self.bins / "voyage-installer").read_bytes())
        root = f"voyage-v1.0.0-{pack.TARGET}"
        with tarfile.open(files[0]) as archive:
            members = archive.getmembers()
            self.assertTrue(all(m.isfile() or m.isdir() for m in members))
            self.assertTrue(all(m.uid == m.gid == m.mtime == 0 for m in members))
            names = {m.name for m in members if m.isfile()}
            expected = {f"{root}/bin/{name}" for name in pack.BINARIES}
            expected |= {f"{root}/{name}" for name in ("README.md", "docs/guide.md", "release.json")}
            for name in pack.BINARIES[:3]:
                expected |= {f"{root}/share/{p}" for p in (
                    f"man/man1/{name}.1", f"bash-completion/completions/{name}",
                    f"zsh/site-functions/_{name}", f"fish/vendor_completions.d/{name}.fish")}
            self.assertEqual(names, expected)
            manifest = json.load(archive.extractfile(root + "/release.json"))
            self.assertEqual(manifest["schema_version"], 1)
            self.assertEqual(manifest["target"], pack.TARGET)
            self.assertEqual(manifest["version"], "v1.0.0")
            self.assertEqual(set(manifest["binaries"]), set(pack.BINARIES))
            for name in pack.BINARIES:
                data = archive.extractfile(f"{root}/bin/{name}").read()
                self.assertEqual(manifest["binaries"][name]["sha256"], hashlib.sha256(data).hexdigest())
                self.assertEqual(archive.getmember(f"{root}/bin/{name}").mode, 0o755)
            text = archive.extractfile(root + "/README.md").read().decode()
            self.assertIn("(docs/guide.md#intro)", text)
            self.assertIn("https://github.com/o-psi/voyage/blob/v1.0.0/src/main.rs", text)
        self.assertNotIn("https:", (self.source / "README.md").read_text())

    def test_existing_assets_and_symlinks_are_preserved(self):
        self.package()
        before = {p.name: p.read_bytes() for p in self.output.iterdir()}
        with self.assertRaises(FileExistsError):
            self.package()
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.output.iterdir()})
        for path in self.output.iterdir():
            path.unlink()
        asset = self.output / f"voyage-installer-{pack.TARGET}.gz.sha256"
        asset.symlink_to("missing")
        with self.assertRaises(FileExistsError):
            self.package()
        self.assertTrue(asset.is_symlink())
        self.assertEqual(list(self.output.iterdir()), [asset])

    def test_foreign_lock_is_preserved(self):
        lock = self.output / ".package-linux.lock"
        lock.mkdir(parents=True)
        with self.assertRaises(FileExistsError):
            self.package()
        self.assertTrue(lock.is_dir())

    def test_partial_publication_rollback_preserves_replacement(self):
        original = os.link
        count = 0
        first = None
        def racing_link(source, destination):
            nonlocal count, first
            count += 1
            if count == 1:
                first = destination
                return original(source, destination)
            if count == 3:
                first.unlink()
                first.write_text("concurrent replacement")
                destination.write_text("concurrent competitor")
                raise FileExistsError("injected race")
            return original(source, destination)
        with patch.object(pack.os, "link", side_effect=racing_link):
            with self.assertRaises(FileExistsError):
                self.package()
        self.assertEqual(sorted(p.read_text() for p in self.output.iterdir()),
                         ["concurrent competitor", "concurrent replacement"])

    def test_bad_version_and_failed_generation_cleanup(self):
        for output in (b"helm 0.9.0\n", b"helm 1.0.0\nextra\n", b""):
            with self.subTest(output=output), patch.object(pack, "run_binary", return_value=output):
                with self.assertRaises(ValueError):
                    pack.package(self.bins, "v1.0.0", self.output, source=self.source)
                self.assert_clean()
        def fail_generation(binary, *args):
            return self.command(binary, *args) if args == ("--version",) else b""
        with patch.object(pack, "run_binary", side_effect=fail_generation):
            with self.assertRaisesRegex(ValueError, "empty generated"):
                pack.package(self.bins, "v1.0.0", self.output, source=self.source)
        self.assert_clean()

    def test_unsafe_missing_and_symlink_documents(self):
        allowlist = self.source / "packaging/linux-documents.txt"
        for text in ("../secret\n", "README.md\nREADME.md\n", "/etc/passwd\n", "missing.md\n"):
            allowlist.write_text(text)
            with self.assertRaises((ValueError, OSError)):
                self.package()
            self.assert_clean()
        allowlist.write_text("docs/guide.md\n")
        shutil.rmtree(self.source / "docs")
        (self.source / "docs").symlink_to(self.bins, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.package()
        self.assert_clean()

    def test_bad_binary_and_label(self):
        for label in ("1.0.0", "../v1.0.0", "v01.0.0", "v1.0.0-rc1"):
            with self.assertRaises(ValueError):
                pack.package(self.bins, label, self.output, source=self.source)
        (self.bins / "helm").write_bytes(b"not ELF")
        with self.assertRaisesRegex(ValueError, "ELF"):
            self.package()
        self.assert_clean()
        (self.bins / "helm").unlink()
        (self.bins / "helm").symlink_to("vessel")
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.package()
        self.assert_clean()

    def test_reference_links_and_escape(self):
        text = "[home]: ../README.md#top\n[code]: <../src/main.rs>\n[external](https://example.org) [anchor](#here)"
        result = pack.release_markdown(text, "docs/guide.md", {"README.md"}, "v1.0.0")
        self.assertIn("[home]: ../README.md#top", result)
        self.assertIn("[code]: <https://github.com/o-psi/voyage/blob/v1.0.0/src/main.rs>", result)
        self.assertIn("[anchor](#here)", result)
        with self.assertRaises(ValueError):
            pack.release_markdown("[bad](../../secret)", "README.md", set(), "v1.0.0")

    def test_checkout_document_links(self):
        names = (pack.ROOT / "packaging/linux-documents.txt").read_text().splitlines()
        # Coordinator-owned notes are an explicit integration dependency, not an
        # optional packager input. Do not synthesize them in this worktree.
        missing = [name for name in names if not (pack.ROOT / name).exists()]
        if missing == ["docs/releases-v1.0.0.md"]:
            self.skipTest("awaiting coordinator release notes; rerun after cherry-pick")
        self.assertEqual(missing, [])
        self.assertEqual(pack.document_paths(pack.ROOT), names)
        for name in names:
            if not name.endswith(".md"):
                continue
            text = pack.release_markdown((pack.ROOT / name).read_text(), name, set(names), "v1.0.0")
            for match in pack.LINK.finditer(text):
                url = (match.group("url") or match.group("refurl")).strip("<>")
                parsed = pack.urlsplit(url)
                if parsed.scheme or parsed.netloc or not parsed.path:
                    continue
                destination = pack.posixpath.normpath(pack.posixpath.join(
                    pack.posixpath.dirname(name), pack.unquote(parsed.path)))
                self.assertIn(destination, names, (name, url))

    def test_real_subprocess_bounds(self):
        executable = self.base / "fixture"
        def script(body):
            executable.write_text("#!/usr/bin/env python3\n" + body)
            executable.chmod(0o755)
        script("print('fixture 1.0.0')\n")
        self.assertEqual(pack.run_binary(executable), b"fixture 1.0.0\n")
        script("import time\ntime.sleep(10)\n")
        with self.assertRaisesRegex(ValueError, "timed out"):
            pack.run_binary(executable, timeout=0.1)
        script("import sys\nsys.stdout.write('x' * 100000)\n")
        with patch.object(pack, "OUTPUT_LIMIT", 1024):
            with self.assertRaisesRegex(ValueError, "output limit"):
                pack.run_binary(executable)
        script("import sys\nsys.exit(7)\n")
        with self.assertRaisesRegex(ValueError, "failed \\(7\\)"):
            pack.run_binary(executable)


if __name__ == "__main__":
    unittest.main()

"""Release fingerprints remain reproducible without the old GX Shell object database."""
import copy
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

import gx_config_fingerprints as fingerprints

ENTRY = re.compile(r'^    \("([^"]+)", (\d+), 0x([0-9a-f]{4}(?:_[0-9a-f]{4}){3}), 0b([01]+)\),$', re.MULTILINE)


def committed() -> list[tuple[str, int, int, int]]:
    text = fingerprints.OUTPUT.read_text(encoding="utf-8")
    return [(path, int(length), int(value.replace("_", ""), 16), int(mask, 2))
            for path, length, value, mask in ENTRY.findall(text)]


class ReleasedConfigTests(unittest.TestCase):
    def test_fingerprint_matches_the_launcher(self):
        self.assertEqual(fingerprints.fnv1a(b""), 0xCBF29CE484222325)
        self.assertEqual(fingerprints.fnv1a(b"a"), 0xAF63DC4C8601EC8C)
        self.assertEqual(fingerprints.fnv1a(b"foobar"), 0x85944171F73967E8)
        self.assertEqual(fingerprints.fingerprint(b"a\r\nb\rc\r\n"), (6, fingerprints.fnv1a(b"a\nb\rc\n")))
        self.assertEqual(fingerprints.fingerprint(b"\0a\r\nb"), (5, fingerprints.fnv1a(b"\0a\r\nb")))

    def test_committed_table_matches_the_registry_without_git(self):
        with patch.object(fingerprints.subprocess, "run", side_effect=AssertionError("must not invoke Git")):
            entries = fingerprints.released_entries()
            self.assertEqual(entries, committed())
            self.assertEqual(fingerprints.render(entries), fingerprints.OUTPUT.read_text(encoding="utf-8"))
            self.assertEqual(fingerprints.main(["--check"]), 0)
        self.assertEqual(entries, sorted(set(entries)))
        self.assertIn(("config/launch.lua", 1935, 0x3A570763D22BE724, 0b001), entries)
        self.assertIn(("config/launch.lua", 2339, 0x7586010E9F4ED8DC, 0b010), entries)
        self.assertFalse({path for path, *_ in entries} & fingerprints.CONFIG_USER_DATA)

    def test_original_releases_keep_every_fingerprint_and_mask(self):
        releases = fingerprints.load_registry()["releases"][:3]
        self.assertEqual([release["commit"] for release in releases], [
            "97d2768b288a16a854f733bfb45ed8f8bf18173a",
            "b0f5d696e5a8b9ce8efa03cb0269525056499c08",
            "ee1905f121c6671f86aa7f3b3761ebb18053cf1a",
        ])
        self.assertEqual([release["tag"] for release in releases],
                         ["gx-v0.3.0", "gx-shell-v0.1.0", "gx-shell-v0.2.0"])
        self.assertEqual([len(release["files"]) for release in releases], [42, 43, 46])
        entries = fingerprints.released_entries(releases)
        self.assertEqual(len(entries), 76)
        self.assertEqual(hashlib.sha256(json.dumps(entries, separators=(",", ":")).encode()).hexdigest(),
                         "8c89d18527efce8dc2786aa024081cd3e33d0ebd6df26e7af5c1d30d6cef0f53")

    def test_source_archive_ignores_git_and_parent_changelog(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            archive = parent / "source"
            for relative in ("scripts/gx_config_fingerprints.py", "scripts/gx_package.py",
                             "scripts/gx-config-releases.json", "scripts/gx-launcher/released.rs"):
                target = archive / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(fingerprints.ROOT / relative, target)
            changelog = parent / "CHANGELOG.md"
            changelog.write_text("## 9.1.0(TBD)\n## 9.0.0(2026-01-01)\n", encoding="utf-8")
            command = [sys.executable, "-B", str(archive / "scripts/gx_config_fingerprints.py"), "--check"]
            env = dict(os.environ, PATH="", PYTHONDONTWRITEBYTECODE="1")
            checked = subprocess.run(command, cwd=parent, env=env, capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            gated = subprocess.run(command + ["--gx-shell-changelog", str(changelog)],
                                   cwd=parent, env=env, capture_output=True, text=True)
            self.assertEqual(gated.returncode, 2)
            self.assertIn("GX Shell 9.0.0", gated.stderr)
            (archive / "scripts/gx-launcher/released.rs").write_text("stale\n", encoding="utf-8")
            stale = subprocess.run(command, cwd=parent, env=env, capture_output=True, text=True)
            self.assertEqual(stale.returncode, 1)
            self.assertIn("is stale", stale.stderr)

    def test_invalid_registry_data_is_rejected(self):
        original = fingerprints.load_registry()
        changes = [
            lambda data: data.update(schema=2),
            lambda data: data.update(algorithm="raw"),
            lambda data: data.update(releases=[]),
            lambda data: data.update(releases=data["releases"] * 3),
            lambda data: data["releases"].append(data["releases"][0]),
            lambda data: data["releases"][0].update(commit="HEAD"),
            lambda data: data["releases"][0].update(tree="123"),
            lambda data: data["releases"][0].update(prefix="../config"),
            lambda data: data["releases"][0].update(files=[]),
            lambda data: data["releases"][0]["files"].reverse(),
            lambda data: data["releases"][0]["files"][0].update(path="../escape"),
            lambda data: data["releases"][0]["files"][0].update(path="/absolute"),
            lambda data: data["releases"][0]["files"][0].update(path='bad".lua'),
            lambda data: data["releases"][0]["files"][0].update(path="gui-settings.json"),
            lambda data: data["releases"][0]["files"][0].update(path="state/runtime"),
            lambda data: data["releases"][0]["files"][0].update(length=-1),
            lambda data: data["releases"][0]["files"][0].update(length=True),
            lambda data: data["releases"][0]["files"][0].update(fnv1a64="wrong"),
            lambda data: data["releases"][0]["files"][0].update(blob="wrong"),
        ]
        for index, change in enumerate(changes):
            with self.subTest(index=index):
                invalid = copy.deepcopy(original)
                change(invalid)
                with self.assertRaises(ValueError):
                    fingerprints.validate_registry(invalid)

    def test_invalid_registry_object_types_are_rejected(self):
        for kind in ("schema", "release", "file"):
            with self.subTest(kind=kind):
                data = fingerprints.load_registry()
                if kind == "schema":
                    data["schema"] = True
                elif kind == "release":
                    data["releases"][0] = None
                else:
                    data["releases"][0]["files"][0] = None
                with self.assertRaises(ValueError):
                    fingerprints.validate_registry(data)

    def test_multiple_shell_releases_can_pin_the_same_component_tag(self):
        data = fingerprints.load_registry()
        before = fingerprints.released_entries(data["releases"])
        release = copy.deepcopy(data["releases"][-1])
        release["name"] = "GX Shell 9.0.0"
        data["releases"].append(release)
        fingerprints.validate_registry(data)
        after = fingerprints.released_entries(data["releases"])
        self.assertEqual([(path, length, value, mask & 0b111) for path, length, value, mask in after], before)

    def test_published_versions_are_the_dated_headings_before_the_newest(self):
        changelog = "## 0.3.0(TBD)\n## 0.2.0(2026-10-01)\n## 0.10.0(TBD)\n## 0.1.0(2026-09-29)\n"
        self.assertEqual(fingerprints.published_versions(changelog), ["0.2.0", "0.1.0"])
        self.assertEqual(fingerprints.published_versions("## 0.2.0(2026-10-01)\n## 0.1.0(2026-09-29)\n"),
                         ["0.1.0"])
        self.assertEqual(fingerprints.published_versions("# Changelog\n"), [])
        self.assertEqual(fingerprints.unlisted_releases(changelog), [])
        self.assertEqual(fingerprints.unlisted_releases(changelog, releases=[]),
                         ["GX Shell 0.2.0", "GX Shell 0.1.0"])


class ReleaseRecordingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "core.autocrlf", "false")
        config = self.root / "dotfiles/wezterm-config"
        config.mkdir(parents=True)
        (config / "wezterm.lua").write_bytes(b"return {}\r\n")
        (config / "image.bin").write_bytes(b"\0binary\r\n")
        (config / "gui-settings.json").write_text("{}", encoding="utf-8")
        (config / "state").mkdir()
        (config / "state/session").write_text("private", encoding="utf-8")
        self.git("add", "--", "dotfiles")
        self.git("commit", "-qm", "fixture")
        self.commit = self.git("rev-parse", "HEAD").strip()
        self.git("tag", "gx-v9.0.0")
        self.metadata = ["WezTerm GX 9.0.0", "https://example.invalid/wezterm",
                         "gx-v9.0.0", self.commit, "dotfiles/wezterm-config"]

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], encoding="utf-8", stderr=subprocess.PIPE)

    def test_record_and_verify_use_immutable_objects_not_the_worktree(self):
        (self.root / "dotfiles/wezterm-config/wezterm.lua").write_text("modified", encoding="utf-8")
        release = fingerprints.collect_release(*self.metadata, self.root)
        files = {entry["path"]: entry for entry in release["files"]}
        self.assertEqual(set(files), {"image.bin", "wezterm.lua"})
        self.assertEqual(files["wezterm.lua"]["length"], len(b"return {}\n"))
        self.assertEqual(files["image.bin"]["length"], len(b"\0binary\r\n"))
        fingerprints.verify_git([release], self.root)
        bad = copy.deepcopy(release)
        bad["files"][0]["length"] += 1
        with self.assertRaisesRegex(ValueError, "differs from Git"):
            fingerprints.verify_git([bad], self.root)

    def test_record_appends_and_preserves_existing_release_bits(self):
        registry = fingerprints.load_registry()
        before = copy.deepcopy(registry["releases"])
        output = self.root / "released.rs"
        record = self.root / "registry.json"
        args = ["--record", "--source-repo", str(self.root)]
        for key, value in zip(("name", "repository", "tag", "commit", "prefix"), self.metadata):
            args.extend([f"--{key}", value])
        with patch.object(fingerprints, "load_registry", return_value=registry), \
                patch.object(fingerprints, "OUTPUT", output), patch.object(fingerprints, "REGISTRY", record), \
                redirect_stdout(io.StringIO()):
            self.assertEqual(fingerprints.main(args), 0)
        actual = fingerprints.load_registry(record)
        self.assertEqual(actual["releases"][:-1], before)
        self.assertEqual(output.read_text(encoding="utf-8"),
                         fingerprints.render(fingerprints.released_entries(actual["releases"]), actual["releases"]))

    def test_explicit_git_audit_fails_if_legacy_objects_are_missing(self):
        with redirect_stderr(io.StringIO()):
            self.assertEqual(fingerprints.main(["--check", "--verify-git", "--source-repo", str(self.root)]), 2)

    def test_record_rejects_a_tag_commit_mismatch(self):
        self.git("commit", "--allow-empty", "-qm", "next")
        self.metadata[3] = self.git("rev-parse", "HEAD").strip()
        args = ["--record", "--source-repo", str(self.root)]
        for key, value in zip(("name", "repository", "tag", "commit", "prefix"), self.metadata):
            args.extend([f"--{key}", value])
        with patch.object(fingerprints, "OUTPUT", self.root / "out.rs"), \
                patch.object(fingerprints, "REGISTRY", self.root / "out.json"), redirect_stderr(io.StringIO()):
            self.assertEqual(fingerprints.main(args), 2)
        self.assertFalse((self.root / "out.rs").exists())
        self.assertFalse((self.root / "out.json").exists())


if __name__ == "__main__":
    unittest.main()

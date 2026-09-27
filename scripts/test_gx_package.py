"""GX packaging contract checks; no compiler, install or network required."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import gx_package as package
import gx_release as release


class PackageTests(unittest.TestCase):
    def test_resource_paths_have_the_same_case_sensitive_order_on_each_host(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("alpha", "Zulu", "Beta"):
                path = root / "dotfiles/plugins" / name / "gitdir/HEAD"
                path.parent.mkdir(parents=True)
                path.write_bytes(b"ref")
            names = [rel.as_posix() for rel, _ in package.snapshot_files(root)]
            self.assertEqual(names, sorted(names))

    def test_semver_uses_numeric_max_and_rejects_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "CHANGELOG.md").write_text("## 0.9.0(2026-01-01)\n## 0.10.0(TBD)\n")
            self.assertEqual(package.version_from_changelog(root), "0.10.0")
            self.assertEqual(package.validate_version("0.10.0", root), "0.10.0")
            for invalid in ("0.9.0", "gx-v0.10.0", "0.10.0;bad", "01.10.0"):
                with self.assertRaises(ValueError):
                    package.validate_version(invalid, root)

    def test_snapshot_excludes_runtime_but_keeps_git_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("plugins/p/gitdir/HEAD", "plugins/p/state/session", "plugins/p/.git/config",
                         "wezterm-config/wezterm.lua", "fonts/example.ttf"):
                path = root / "dotfiles" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"example")
            names = {rel.as_posix() for rel, _ in package.snapshot_files(root)}
            self.assertEqual(names, {"plugins/p/gitdir/HEAD", "wezterm-config/wezterm.lua", "fonts/example.ttf"})
            first = package.resource_version(root)
            (root / "dotfiles/plugins/p/state/session").write_bytes(b"personal session")
            self.assertEqual(package.resource_version(root), first)
            (root / "dotfiles/plugins/p/gitdir/HEAD").write_bytes(b"updated")
            self.assertNotEqual(package.resource_version(root), first)

    def test_all_shipped_fonts_have_registration_names(self):
        fonts = list((package.ROOT / "dotfiles/fonts").glob("*.tt*"))
        self.assertGreaterEqual(len(fonts), 8)
        for path in fonts:
            self.assertTrue(package.font_name(path))

    def test_missing_and_foreign_binaries_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "missing binary"):
                package.verify_binaries(root, "deb", "a" * 40)
            (root / "wezterm").write_bytes(b"MZ" + b"\0" * 62)
            with self.assertRaisesRegex(ValueError, "amd64 ELF"):
                package.verify_binaries(root, "deb", "a" * 40)

    def test_linux_abi_rejects_new_glibc_and_system_openssl(self):
        binary = Path("wezterm-gui")
        with patch.object(package, "tool", return_value=Path("objdump")):
            for symbols in ("GLIBC_2.34", "GLIBC_2.9 GLIBC_2.32", "GLIBC_PRIVATE", ""):
                with patch.object(package, "output", return_value=symbols):
                    with self.assertRaisesRegex(ValueError, "glibc"):
                        package.audit_linux_binary(binary)
            for soname in ("libssl.so.1.1", "libcrypto.so.3"):
                with patch.object(package, "output", side_effect=["GLIBC_2.31", f"  NEEDED {soname}"]):
                    with self.assertRaisesRegex(ValueError, "OpenSSL"):
                        package.audit_linux_binary(binary)
            with patch.object(package, "output", side_effect=["GLIBC_2.9 GLIBC_2.31", "  NEEDED libc.so.6"]):
                self.assertEqual(package.audit_linux_binary(binary),
                                 {"max_glibc": "2.31", "needed": ["libc.so.6"]})


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sha = "a" * 40
        for name, platform in (("WezTerm-GX-1.2.3-Setup-x64.exe", "windows"),
                               ("wezterm-gx_1.2.3_amd64.deb", "deb")):
            artifact = self.root / name
            artifact.write_bytes(b"installer-fixture")
            info = {"schema": 1, "package_version": "1.2.3", "source_commit": self.sha,
                    "source_dirty": False, "product_version": "20260927-120000-aaaaaaaa",
                    "resource_version": "b" * 64, "platform": platform, "architecture": "amd64",
                    "artifact": name, "sha256": package.digest(artifact)}
            if platform == "deb":
                info["linux_compatibility"] = {
                    "build_ubuntu": "20.04", "supported_ubuntu": ["20.04", "24.04"], "openssl": "static",
                    "elf": {name: {"max_glibc": "2.31", "needed": ["libc.so.6"]}
                            for name in (*package.BINARIES, "wezterm-gx", "wezterm-gx-gui")},
                }
            artifact.with_name(name + ".manifest.json").write_text(json.dumps(info))
            self.refresh_checksums(name)

    def refresh_checksums(self, name):
        artifact = self.root / name
        metadata = self.root / (name + ".manifest.json")
        (self.root / (name + ".sha256")).write_text(
            f"{package.digest(artifact)}  {name}\n{package.digest(metadata)}  {metadata.name}\n", encoding="ascii")

    def test_both_platforms_are_required(self):
        self.assertEqual(len(release.verify_artifacts(self.root, "1.2.3", self.sha)), 6)
        (self.root / "wezterm-gx_1.2.3_amd64.deb").unlink()
        with self.assertRaisesRegex(ValueError, "missing release"):
            release.verify_artifacts(self.root, "1.2.3", self.sha)

    def test_tampered_artifact_is_rejected(self):
        (self.root / "wezterm-gx_1.2.3_amd64.deb").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            release.verify_artifacts(self.root, "1.2.3", self.sha)

    def test_dirty_or_different_source_is_rejected(self):
        path = self.root / "wezterm-gx_1.2.3_amd64.deb.manifest.json"
        original = json.loads(path.read_text())
        for key, value in (("source_dirty", True), ("source_commit", "c" * 40),
                           ("resource_version", "c" * 64), ("package_version", "1.2.4")):
            path.write_text(json.dumps(dict(original, **{key: value})))
            self.refresh_checksums("wezterm-gx_1.2.3_amd64.deb")
            with self.assertRaises(ValueError):
                release.verify_artifacts(self.root, "1.2.3", self.sha)

    def test_tag_cannot_be_moved(self):
        release.verify_tag(None, self.sha)
        release.verify_tag(self.sha, self.sha)
        with self.assertRaises(ValueError):
            release.verify_tag("c" * 40, self.sha)

    def test_release_rejects_incompatible_or_incomplete_linux_audit(self):
        path = self.root / "wezterm-gx_1.2.3_amd64.deb.manifest.json"
        original = path.read_text()
        for change in ("missing", "new-glibc", "dynamic-openssl", "missing-launcher"):
            info = json.loads(original)
            audit = info["linux_compatibility"]
            if change == "missing":
                del info["linux_compatibility"]
            elif change == "new-glibc":
                audit["elf"]["wezterm-gx"]["max_glibc"] = "2.39"
            elif change == "dynamic-openssl":
                audit["elf"]["wezterm"]["needed"].append("libssl.so.1.1")
            else:
                del audit["elf"]["wezterm-gx-gui"]
            path.write_text(json.dumps(info))
            self.refresh_checksums("wezterm-gx_1.2.3_amd64.deb")
            with self.assertRaises(ValueError):
                release.verify_artifacts(self.root, "1.2.3", self.sha)

    def test_local_publish_never_contacts_network(self):
        with patch.dict("os.environ", {}, clear=True), patch.object(release, "GitHub") as api:
            with self.assertRaisesRegex(ValueError, "manual"):
                release.publish(self.root, "1.2.3", self.sha)
            api.assert_not_called()

    def publish_with(self, api):
        env = {"GITHUB_ACTIONS": "true", "GITHUB_EVENT_NAME": "workflow_dispatch",
               "GITHUB_REPOSITORY": release.REPOSITORY}
        with patch.dict("os.environ", env), patch.object(release, "GitHub", return_value=api):
            release.publish(self.root, "1.2.3", self.sha)

    def test_published_release_and_conflicting_tag_never_write(self):
        for published, commit in ((True, self.sha), (False, "c" * 40)):
            api = FakeGitHub(commit, published=published)
            with self.assertRaises(ValueError):
                self.publish_with(api)
            self.assertFalse(api.writes)

    def test_single_platform_failure_never_creates_tag(self):
        (self.root / "wezterm-gx_1.2.3_amd64.deb").unlink()
        api = FakeGitHub(self.sha)
        with self.assertRaisesRegex(ValueError, "missing release artifact"):
            self.publish_with(api)
        self.assertFalse(api.writes)

    def test_failed_upload_keeps_draft_unpublished(self):
        api = FakeGitHub(None, fail_upload=True)
        with self.assertRaises(OSError):
            self.publish_with(api)
        self.assertFalse(any(method == "PATCH" for method, _ in api.writes))
        self.assertTrue(api.draft)

    def test_publish_requires_all_six_remote_assets(self):
        api = FakeGitHub(None)
        self.publish_with(api)
        self.assertEqual(len(api.assets), 6)
        self.assertEqual(api.writes[-1], ("PATCH", "/releases/1"))
        self.assertFalse(api.draft)

    def test_build_only_verification_never_contacts_network(self):
        with patch.object(release, "GitHub") as api:
            release.verify_artifacts(self.root, "1.2.3", self.sha)
            api.assert_not_called()


class FakeGitHub:
    """In-memory service for publication failure/ordering tests."""
    def __init__(self, commit, published=False, fail_upload=False):
        self.commit, self.published, self.fail_upload = commit, published, fail_upload
        self.writes, self.assets = [], []
        self.draft = True

    def optional(self, path):
        if path.startswith("/git/ref/"):
            return {"object": {"type": "commit", "sha": self.commit}} if self.commit else None
        return {"draft": False} if self.published else None

    def request(self, method, path, data=None, binary=None):
        if method != "GET":
            self.writes.append((method, path))
        if path == "/git/refs":
            self.commit = data["sha"]
        elif path == "/releases":
            self.draft = data["draft"]
            return {"id": 1, "upload_url": "https://uploads.github.com/test{?name}"}
        elif path.startswith("https://uploads.github.com/"):
            if self.fail_upload:
                raise OSError("simulated interrupted upload")
            self.assets.append({"name": binary.name, "state": "uploaded"})
        elif method == "GET":
            return self.assets
        elif method == "PATCH":
            self.draft = data["draft"]
            return {"html_url": "https://github.com/gx0404/wezterm/releases/tag/gx-v1.2.3"}


if __name__ == "__main__":
    unittest.main()

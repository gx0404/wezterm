"""GX packaging contract checks; no compiler, install or network required."""
import io
import json
import subprocess
import tempfile
import unittest
import urllib.error
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import gx_package as package
import gx_release as release


class PackageTests(unittest.TestCase):
    def test_explicit_inno_compiler_wins_over_preinstalled_path(self):
        with tempfile.TemporaryDirectory() as directory:
            pinned = Path(directory) / "ISCC.exe"
            pinned.write_bytes(b"fixture")
            with patch.dict("os.environ", {"ISCC": str(pinned)}), \
                    patch.object(package.shutil, "which", return_value="old-inno.exe"):
                self.assertEqual(package.tool("iscc"), pinned)
                pinned.unlink()
                with self.assertRaisesRegex(ValueError, "ISCC points to a missing"):
                    package.tool("iscc")

    def test_inno_preflight_rejects_old_compilers_before_building(self):
        with patch.object(package, "tool", return_value=Path("ISCC.exe")):
            for version in ("6.7.1", "6.7.3", "7.0.0", "unknown"):
                with patch.object(package, "output", return_value=version):
                    with self.assertRaisesRegex(ValueError, "long plugin paths"):
                        package.verify_inno()
            with patch.object(package, "output", side_effect=subprocess.CalledProcessError(1, "ISCC")):
                with self.assertRaisesRegex(ValueError, "Inno Setup >= 7.1"):
                    package.verify_inno()
            with patch.object(package, "output", return_value="7.1.0"):
                package.verify_inno()

    def test_failed_container_build_returns_cache_ownership_without_touching_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in ("get-deps", "ci/check-rust-version.sh", "scripts/packaging/ubuntu2004.Dockerfile"):
                source = root / relative
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text("fixture\n")
            args = SimpleNamespace(check=False, bin_dir=None, output_dir=root / "dist", cache_dir=root / "cache",
                                   stage_dir=None)
            calls = []

            def docker_run(command):
                calls.append(list(map(str, command)))
                if "python3" in command:
                    raise subprocess.CalledProcessError(1, "container-build")

            with patch.object(package, "ROOT", root), patch.object(package.sys, "platform", "linux"), \
                    patch.object(package.platform, "machine", return_value="x86_64"), \
                    patch.object(package, "tool", return_value="docker"), \
                    patch.object(package, "output", side_effect=["linux", str(root), ".git"]), \
                    patch.object(package.os, "getuid", return_value=1001, create=True), \
                    patch.object(package.os, "getgid", return_value=1001, create=True), \
                    patch.object(package, "run", side_effect=docker_run):
                with self.assertRaises(subprocess.CalledProcessError):
                    package.container_deb(args, "1.2.3")
            handoff = calls[-1]
            self.assertIn("--no-dereference", handoff)
            self.assertEqual(handoff[handoff.index("1001:1001") + 1:],
                             ["/gx-owned-cache-0", "/gx-owned-cache-1", "/gx-owned-cache-2"])
            self.assertNotIn(str(root), handoff)

    def test_monorepo_container_mounts_whole_tree_and_returns_stage_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            top = Path(directory).resolve() / "gx_shell"
            root = top / "wezterm"
            for relative in ("get-deps", "ci/check-rust-version.sh", "scripts/packaging/ubuntu2004.Dockerfile"):
                source = root / relative
                source.parent.mkdir(parents=True, exist_ok=True)
                source.write_text("fixture\n")
            (top / ".git").mkdir()
            stage = Path(directory).resolve() / "stages" / "wezterm"
            args = SimpleNamespace(check=False, bin_dir=None, output_dir=top / "dist", cache_dir=None,
                                   stage_dir=stage)
            calls = []

            def docker_run(command):
                calls.append(list(map(str, command)))
                if "python3" in command:
                    stage.mkdir()

            with patch.object(package, "ROOT", root), patch.object(package.sys, "platform", "linux"), \
                    patch.object(package.platform, "machine", return_value="x86_64"), \
                    patch.object(package, "tool", return_value="docker"), \
                    patch.object(package, "output", side_effect=["linux", str(top), "../.git"]), \
                    patch.object(package.os, "getuid", return_value=1001, create=True), \
                    patch.object(package.os, "getgid", return_value=1001, create=True), \
                    patch.object(package, "run", side_effect=docker_run):
                package.container_deb(args, "1.2.3")
            build = next(call for call in calls if "python3" in call)
            volumes = [build[i + 1] for i, arg in enumerate(build) if arg == "--volume"]
            self.assertIn(f"{top}:{top}", volumes)
            self.assertIn(f"{stage.parent}:{stage.parent}", volumes)
            self.assertFalse(any(volume.startswith(f"{top / '.git'}:") for volume in volumes))
            self.assertEqual(build[build.index("--stage-dir") + 1], str(stage))
            handoff = calls[-1]
            self.assertIn(f"{stage}:/gx-owned-stage", handoff)
            self.assertEqual(handoff[-1], "/gx-owned-stage")

    def test_dirty_check_is_scoped_to_the_wezterm_tree(self):
        root = Path("wezterm").resolve()
        with patch.object(package, "output", side_effect=[str(root), "a" * 40, ""]) as git:
            self.assertEqual(package.source_info(root), ("a" * 40, False))
        self.assertEqual(git.call_args_list[2].args[0][-2:], ["--", "."])

    def test_source_info_rejects_parent_repository(self):
        root = Path("wezterm").resolve()
        with patch.object(package, "output", return_value=str(root.parent)):
            with self.assertRaisesRegex(ValueError, "independent Git checkout"):
                package.source_info(root)

    def test_stage_dir_must_be_new_before_anything_builds(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(package.sys, "argv", ["gx_package.py", "windows", "--stage-dir", directory]), \
                patch.object(package, "preflight") as preflight, \
                patch("sys.stderr", new_callable=io.StringIO) as stderr:
            self.assertEqual(package.main(), 1)
        preflight.assert_not_called()
        self.assertIn("--stage-dir must not exist", stderr.getvalue())

    def test_stage_only_windows_preflight_does_not_need_inno(self):
        seen = []

        def fake_tool(name):
            seen.append(name)
            return name

        bin_dir = Path("prebuilt")
        with patch.object(package.os, "name", "nt"), \
                patch.object(package.platform, "machine", return_value="AMD64"), \
                patch.object(package, "tool", side_effect=fake_tool), \
                patch.object(package, "verify_inno") as inno, \
                patch.object(package, "output", return_value="rustc 1.96.1 (fixture 2026-01-01)"), \
                patch("sys.stdout", new_callable=io.StringIO):
            package.preflight("windows", bin_dir, stage_only=True)
        self.assertNotIn("iscc", seen)
        inno.assert_not_called()

    def test_windows_stage_holds_the_app_payload_and_fonts_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "wezterm"
            files = ["LICENSE.md", "dotfiles/PROVENANCE.md", "dotfiles/README.md",
                     "dotfiles/wezterm-config/wezterm.lua", "dotfiles/plugins/p/gitdir/HEAD",
                     "dotfiles/fonts/A.ttf", "dotfiles/fonts/B.ttc", "dotfiles/fonts/notes.txt",
                     *[f"assets/windows/{source}" for source in package.WINDOWS_RUNTIME]]
            for relative in files:
                (root / relative).parent.mkdir(parents=True, exist_ok=True)
                (root / relative).write_bytes(b"fixture")
            bin_dir = Path(directory) / "bin"
            bin_dir.mkdir()
            for name in package.BINARIES:
                (bin_dir / f"{name}.exe").write_bytes(b"MZ")
            stage = Path(directory) / "stage"
            stage.mkdir()

            def launchers(dest, kind):
                for name in ("wezterm-gx.exe", "wezterm-gx-cli.exe"):
                    (dest / name).write_bytes(b"MZ")

            with patch.object(package, "compile_launchers", side_effect=launchers):
                fonts = package.stage_windows(stage, bin_dir, root)
            staged = {path.relative_to(stage).as_posix() for path in stage.rglob("*") if path.is_file()}
            expected = {f"app/{name}.exe" for name in package.BINARIES}
            expected |= {f"app/{target}" for target in package.WINDOWS_RUNTIME.values()}
            expected |= {"app/LICENSE.md", "app/wezterm-gx.exe", "app/wezterm-gx-cli.exe",
                         "app/resources/resource-version", "app/resources/dotfiles/PROVENANCE.md",
                         "app/resources/dotfiles/README.md", "app/resources/dotfiles/wezterm-config/wezterm.lua",
                         "app/resources/dotfiles/plugins/p/gitdir/HEAD", "fonts/A.ttf", "fonts/B.ttc"}
            self.assertEqual(staged, expected)
            self.assertEqual([path.name for path in fonts], ["A.ttf", "B.ttc"])

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
            for automatic in (None, "", "  ", " 0.10.0 "):
                self.assertEqual(package.validate_version(automatic, root), "0.10.0")
            for invalid in ("0.9.0", "gx-v0.10.0", "0.10.0;bad", "01.10.0"):
                with self.assertRaises(ValueError):
                    package.validate_version(invalid, root)
            with self.assertRaisesRegex(ValueError, r"0\.9\.0.*0\.10\.0"):
                package.validate_version("0.9.0", root)

    def test_product_version_uses_utc_commit_time(self):
        with patch.object(package, "output", return_value="0"):
            self.assertEqual(package.product_version_from_source("a" * 40), "19700101-000000-aaaaaaaa")

    def test_snapshot_excludes_runtime_but_keeps_git_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("plugins/p/gitdir/HEAD", "plugins/p/state/session", "plugins/p/.git/config",
                         "wezterm-config/wezterm.lua", "wezterm-config/gui-settings.json", "fonts/example.ttf"):
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


class PreflightPathTests(unittest.TestCase):
    def test_windows_preflight_rejects_non_ascii_root(self):
        with patch.object(package, "ROOT", Path("D:/\u5de5\u7a0b/wezterm")), \
                patch.object(package.os, "name", "nt"), \
                patch.object(package.platform, "machine", return_value="AMD64"):
            with self.assertRaisesRegex(ValueError, "pure ASCII"):
                package.preflight("windows", None)

    def test_windows_preflight_passes_ascii_check_for_ascii_root(self):
        with patch.object(package, "ROOT", Path("D:/gx_projects/wezterm")), \
                patch.object(package.os, "name", "nt"), \
                patch.object(package.platform, "machine", return_value="AMD64"), \
                patch.object(package, "tool", side_effect=ValueError("stop after ascii check")):
            with self.assertRaisesRegex(ValueError, "stop after ascii check"):
                package.preflight("windows", None)


class TagCleanupTests(unittest.TestCase):
    def _main(self, root, run_effect):
        argv = ["gx_package.py", "windows", "--version", "1.2.3"]
        with patch.object(package, "ROOT", root), patch.object(package.sys, "argv", argv), \
                patch.object(package, "validate_version", return_value="1.2.3"), \
                patch.object(package, "preflight"), \
                patch.object(package, "source_info", return_value=("a" * 40, False)), \
                patch.object(package, "product_version_from_source", return_value="GX-fixture"), \
                patch.object(package, "tool", return_value=Path("cargo")), \
                patch.object(package, "run", side_effect=run_effect), \
                patch.dict("os.environ", {"CARGO_TARGET_DIR": str(root / "target")}), \
                patch("sys.stderr", new_callable=io.StringIO):
            return package.main()

    def test_created_tag_is_removed_after_failed_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            seen = []
            def fail(*args, **kwargs):
                seen.append((root / ".tag").is_file())
                raise subprocess.CalledProcessError(1, "cargo")
            self.assertEqual(self._main(root, fail), 1)
            self.assertEqual(seen, [True])
            self.assertFalse((root / ".tag").exists())

    def test_preexisting_tag_is_kept_after_failed_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".tag").write_text("keep-me\n", encoding="ascii")
            def fail(*args, **kwargs):
                raise subprocess.CalledProcessError(1, "cargo")
            self.assertEqual(self._main(root, fail), 1)
            self.assertTrue((root / ".tag").is_file())


class StageTests(unittest.TestCase):
    def make_stage(self, root, kind='windows'):
        stage = root / 'stage'
        stage.mkdir()
        prefix = 'app' if kind == 'windows' else 'root/usr/lib/wezterm-gx'
        binaries = {}
        for name in package.BINARIES:
            name += '.exe' if kind == 'windows' else ''
            path = stage / prefix / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'fixture binary\r\n')
            binaries[name] = package.digest(path)
        payload = stage / prefix / 'config.lua'
        payload.write_bytes(b'local value = 1\r\n')
        package.stage_build_inputs(stage, kind)
        manifest = {
            'schema': 1, 'platform': kind, 'architecture': 'amd64',
            'source_commit': 'a' * 40, 'source_dirty': True,
            'package_version': '1.2.3', 'product_version': 'fixture-version',
            'resource_version': 'b' * 64, 'binaries': binaries,
        }
        package.write_stage_manifest(stage, manifest)
        return stage

    def rewrite_manifest(self, stage, transform):
        path = stage / package.STAGE_MANIFEST
        manifest = json.loads(path.read_text(encoding='utf-8'))
        transform(manifest)
        path.write_text(json.dumps(manifest), encoding='utf-8')

    def test_stage_contract_covers_raw_bytes_and_build_inputs(self):
        for kind in ('windows', 'deb'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp), kind)
                manifest = package.verify_stage(stage)
                self.assertEqual(manifest['schema'], 2)
                self.assertEqual(manifest['source_repository'], 'gx0404/wezterm')
                self.assertEqual(manifest['source_commit'], 'a' * 40)
                self.assertIs(manifest['source_dirty'], True)
                self.assertEqual(manifest['package_version'], '1.2.3')
                files = {entry['path']: entry for entry in manifest['files']}
                actual = {path.relative_to(stage).as_posix() for path in stage.rglob('*')
                          if path.is_file() and path.name != package.STAGE_MANIFEST}
                self.assertEqual(set(files), actual)
                self.assertIn('build-inputs/gx-config-releases.json', files)
                self.assertEqual('build-inputs/terminal.ico' in files, kind == 'windows')
                for name, entry in files.items():
                    data = (stage / name).read_bytes()
                    self.assertEqual(entry['size'], len(data))
                    self.assertEqual(entry['sha256'], package.hashlib.sha256(data).hexdigest())
                self.assertEqual((stage / 'build-inputs/gx-config-releases.json').read_bytes(),
                                 (package.ROOT / 'scripts/gx-config-releases.json').read_bytes())

    def test_stage_rejects_tampering_missing_and_extra_files(self):
        for operation in ('tamper', 'missing', 'extra'):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp))
                path = stage / 'app/config.lua'
                if operation == 'tamper':
                    path.write_bytes(path.read_bytes().replace(b'1', b'2'))
                elif operation == 'missing':
                    path.unlink()
                else:
                    (stage / 'unexpected.txt').write_bytes(b'extra')
                with self.assertRaises(ValueError):
                    package.verify_stage(stage)

    def test_stage_rejects_unsafe_manifest_paths(self):
        for path in ('../escape', '/absolute', 'a//b', 'a/./b', 'C:/escape',
                     'a' + chr(92) + 'b', '', 'a' + chr(0), 'a./b', 'a /b'):
            with self.subTest(path=path), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp))
                self.rewrite_manifest(stage, lambda manifest: manifest['files'][0].update(path=path))
                with self.assertRaises(ValueError):
                    package.verify_stage(stage)

    def test_stage_rejects_invalid_and_duplicate_inventory_entries(self):
        mutations = {
            'duplicate': lambda m: m['files'].append(dict(m['files'][0])),
            'case_collision': lambda m: m['files'].append(dict(m['files'][0], path=m['files'][0]['path'].upper())),
            'invalid_hash': lambda m: m['files'][0].update(sha256='not-a-hash'),
            'boolean_size': lambda m: m['files'][0].update(size=True),
            'unknown_field': lambda m: m['files'][0].update(unexpected=True),
            'self_inventory': lambda m: m['files'][0].update(path=package.STAGE_MANIFEST),
            'wrong_schema': lambda m: m.update(schema=1),
            'wrong_repository': lambda m: m.update(source_repository='other/wezterm'),
            'unsafe_link': lambda m: m['files'].append({'path': 'root/usr/bin/wezterm-gx', 'symlink': '../../escape'}),
        }
        for name, mutation in mutations.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp))
                self.rewrite_manifest(stage, mutation)
                with self.assertRaises(ValueError):
                    package.verify_stage(stage)

    def test_stage_requires_exported_build_inputs(self):
        for kind, name in (('windows', 'terminal.ico'), ('windows', 'gx-config-releases.json'),
                           ('deb', 'gx-config-releases.json')):
            with self.subTest(kind=kind, name=name), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp), kind)
                (stage / 'build-inputs' / name).unlink()
                self.rewrite_manifest(stage, lambda m: m.update(files=package.stage_inventory(stage, kind)))
                with self.assertRaises(ValueError):
                    package.verify_stage(stage)

    def test_linux_entry_symlinks_are_fixed_and_resolve_to_regular_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage = self.make_stage(Path(tmp), 'deb')
            for name, target in package.STAGE_LINKS.items():
                path = stage / name
                path.parent.mkdir(parents=True, exist_ok=True)
                (stage / 'root/usr/lib/wezterm-gx' / path.name).write_bytes(b'launcher')
                try:
                    path.symlink_to(target)
                except OSError as error:
                    self.skipTest(f'host does not permit symlink creation: {error}')
            manifest = json.loads((stage / package.STAGE_MANIFEST).read_text(encoding='utf-8'))
            package.write_stage_manifest(stage, manifest)
            verified = package.verify_stage(stage)
            links = {entry['path']: entry['symlink'] for entry in verified['files'] if 'symlink' in entry}
            self.assertEqual(links, package.STAGE_LINKS)
            path = stage / next(iter(package.STAGE_LINKS))
            path.unlink()
            path.symlink_to('../../escape')
            with self.assertRaises(ValueError):
                package.verify_stage(stage)

    def test_stage_rejects_unapproved_and_directory_symlinks(self):
        for kind in ('windows', 'deb'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                stage = self.make_stage(Path(tmp), kind)
                path = stage / 'unapproved'
                try:
                    path.symlink_to(stage / 'build-inputs', target_is_directory=True)
                except OSError as error:
                    self.skipTest(f'host does not permit symlink creation: {error}')
                with self.assertRaises(ValueError):
                    package.verify_stage(stage)

    def test_verify_stage_cli_runs_without_git_or_build_tools(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            stage = self.make_stage(root)
            command = [package.sys.executable, '-B', str(package.ROOT / 'scripts/gx_package.py'),
                       'verify-stage', '--stage', str(stage)]
            env = dict(package.os.environ, PATH='')
            result = subprocess.run(command, cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('PASS stage:', result.stdout)
            (stage / 'unexpected.txt').write_bytes(b'extra')
            result = subprocess.run(command, cwd=root, env=env, text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('ERROR:', result.stderr)


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
            info["binaries"] = {binary + (".exe" if platform == "windows" else ""): "c" * 64
                                for binary in package.BINARIES}
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

    def test_empty_artifact_is_rejected(self):
        (self.root / "wezterm-gx_1.2.3_amd64.deb").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "empty release artifact"):
            release.verify_artifacts(self.root, "1.2.3", self.sha)

    def test_malformed_manifest_fields_fail_closed(self):
        path = self.root / "wezterm-gx_1.2.3_amd64.deb.manifest.json"
        original = json.loads(path.read_text())
        for key, value in (("schema", True), ("source_dirty", 0), ("binaries", {}),
                           ("binaries", {name: "bad" for name in package.BINARIES}),
                           ("resource_version", []), ("product_version", None),
                           ("linux_compatibility", [])):
            with self.subTest(key=key, value=value):
                path.write_text(json.dumps(dict(original, **{key: value})))
                self.refresh_checksums("wezterm-gx_1.2.3_amd64.deb")
                with self.assertRaises(ValueError):
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
        with patch.dict("os.environ", env, clear=True), patch.object(release, "GitHub", return_value=api):
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

    def test_remote_size_digest_and_upload_state_must_match(self):
        for key, value in (("size", 0), ("digest", "sha256:" + "0" * 64), ("state", "starter")):
            api = FakeGitHub(None, corrupt={key: value})
            with self.assertRaisesRegex(ValueError, "size/SHA-256"):
                self.publish_with(api)
            self.assertTrue(api.draft)
            self.assertFalse(any(method == "PATCH" for method, _ in api.writes))

    def test_interrupted_draft_resumes_identical_assets_only(self):
        api = FakeGitHub(self.sha)
        api.exists = True
        path = self.root / "WezTerm-GX-1.2.3-Setup-x64.exe"
        api.assets.append(FakeGitHub.asset(path))
        self.publish_with(api)
        uploads = [path for method, path in api.writes if path.startswith("https://uploads")]
        self.assertEqual(len(uploads), 5)
        self.assertFalse(api.draft)

    def test_unexpected_draft_asset_prevents_any_upload(self):
        api = FakeGitHub(self.sha)
        api.exists = True
        api.assets.append({"name": "unrelated-file"})
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.publish_with(api)
        self.assertFalse(api.writes)

    def test_prepare_checks_publication_conflicts_without_writes(self):
        for published, commit in ((True, self.sha), (False, "c" * 40)):
            api = FakeGitHub(commit, published=published)
            with patch.object(release, "source_info", return_value=(self.sha, False)), \
                    patch.object(release, "GitHub", return_value=api):
                with self.assertRaises(ValueError):
                    release.prepare("1.2.3", publish_requested=True)
            self.assertFalse(api.writes)

    def test_build_only_prepare_ignores_remote_tag_conflicts(self):
        with patch.dict("os.environ", {}, clear=True), \
                patch.object(release, "source_info", return_value=(self.sha, False)), \
                patch.object(release, "GitHub") as api:
            release.prepare("1.2.3", publish_requested=False)
            api.assert_not_called()

    def test_build_only_verification_never_contacts_network(self):
        with patch.object(release, "GitHub") as api:
            release.verify_artifacts(self.root, "1.2.3", self.sha)
            api.assert_not_called()


class FakeGitHub:
    """In-memory service for publication failure/ordering tests."""
    def __init__(self, commit, published=False, fail_upload=False, corrupt=None):
        self.commit, self.published, self.fail_upload = commit, published, fail_upload
        self.writes, self.assets = [], []
        self.draft = True
        self.exists = published
        self.corrupt = corrupt or {}

    @staticmethod
    def asset(path):
        return {"name": path.name, "state": "uploaded", "size": path.stat().st_size,
                "digest": "sha256:" + package.digest(path)}

    def release(self):
        return {"id": 1, "draft": self.draft and not self.published,
                "upload_url": "https://uploads.github.com/test{?name}"}

    def optional(self, path):
        if path.startswith("/git/ref/"):
            return {"object": {"type": "commit", "sha": self.commit}} if self.commit else None
        return self.release() if self.exists else None

    def request(self, method, path, data=None, binary=None):
        if method != "GET":
            self.writes.append((method, path))
        if path == "/git/refs":
            self.commit = data["sha"]
        elif path == "/releases":
            self.draft = data["draft"]
            self.exists = True
            return self.release()
        elif path.startswith("https://uploads.github.com/"):
            if self.fail_upload:
                raise OSError("simulated interrupted upload")
            self.assets.append(dict(self.asset(binary), **self.corrupt))
        elif method == "GET":
            return self.release() if path == "/releases/1" else self.assets
        elif method == "PATCH":
            self.draft = data["draft"]
            return {"html_url": "https://github.com/gx0404/wezterm/releases/tag/gx-v1.2.3"}


class GitHubTransportTests(unittest.TestCase):
    def test_only_transient_reads_are_retried(self):
        for method, code, attempts in (("GET", 503, 3), ("GET", 404, 1), ("POST", 503, 1)):
            with patch.dict("os.environ", {"GH_TOKEN": "test"}), \
                    patch.object(release.urllib.request, "urlopen") as open_url, \
                    patch.object(release.time, "sleep"):
                open_url.side_effect = urllib.error.HTTPError("https://api.github.com/test", code, "failure", {}, io.BytesIO())
                with self.assertRaises(urllib.error.HTTPError) as raised:
                    release.GitHub().request(method, "/releases")
                raised.exception.close()
                self.assertEqual(open_url.call_count, attempts)


if __name__ == "__main__":
    unittest.main()

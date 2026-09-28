import os
import platform
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CRATES = ("wezterm", "wezterm-gui", "wezterm-mux-server")
PRAGMA = "#pragma code_page(65001)"
VERSION = "20990101-000000-resource-test"


def resource_template(crate):
    source = (ROOT / crate / "build.rs").read_text(encoding="utf-8")
    templates = re.findall(r'r#"(.*?)"#,', source, re.S)
    resources = [text for text in templates if "RT_MANIFEST" in text]
    if len(resources) != 1:
        raise AssertionError(f"Expected one resource template in {crate}/build.rs")
    return resources[0]


def find_windows_sdk():
    roots = []
    if os.environ.get("WindowsSdkDir"):
        roots.append(Path(os.environ["WindowsSdkDir"]))
    for variable in ("ProgramFiles(x86)", "ProgramFiles"):
        if os.environ.get(variable):
            roots.append(Path(os.environ[variable]) / "Windows Kits" / "10")
    machine = platform.machine().lower()
    host = "arm64" if machine == "arm64" else "x64" if machine == "amd64" else "x86"
    for root in dict.fromkeys(roots):
        versions = [path for path in (root / "bin").glob("*")
                    if re.fullmatch(r"\d+(?:\.\d+)+", path.name)]
        versions.sort(key=lambda path: tuple(map(int, path.name.split("."))), reverse=True)
        for version in versions:
            include = root / "Include" / version.name
            includes = [include / "um", include / "shared"]
            if not (includes[0] / "winres.h").is_file() or not includes[1].is_dir():
                continue
            for arch in dict.fromkeys((host, "x64", "x86")):
                rc = version / arch / "rc.exe"
                if rc.is_file():
                    return rc, includes
    rc = shutil.which("rc.exe")
    includes = [Path(path) for path in os.environ.get("INCLUDE", "").split(";") if path]
    if rc and any((path / "winres.h").is_file() for path in includes):
        return Path(rc), includes
    raise unittest.SkipTest("Windows SDK rc.exe and winres.h are not installed")


class ResourceTemplateTests(unittest.TestCase):
    def test_utf8_selected_before_headers_and_paths(self):
        for crate in CRATES:
            with self.subTest(crate=crate):
                template = resource_template(crate)
                self.assertEqual(template.count(PRAGMA), 1)
                self.assertEqual(template.strip().splitlines()[0], PRAGMA)
                self.assertLess(template.index(PRAGMA), template.index("#include"))
                self.assertLess(template.index(PRAGMA), template.index("{win}"))


@unittest.skipUnless(os.name == "nt", "Requires the Windows resource compiler")
class WindowsResourceCompilerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rc, cls.includes = find_windows_sdk()

    def setUp(self):
        temporary_root = ROOT / ".local"
        temporary_root.mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="windows-resources-项目 ", dir=temporary_root)
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.assets = self.directory / "资源 files"
        self.assets.mkdir()
        for name in ("console.manifest", "manifest.manifest", "terminal.ico"):
            shutil.copyfile(ROOT / "assets/windows" / name, self.assets / name)

    def compile_resource(self, crate, suffix, *, pragma=True, code_page=None):
        template = resource_template(crate)
        if not pragma:
            template = template.replace(PRAGMA, "")
        source = template.replace("{win}", str(self.assets).replace("\\", "\\\\"))
        source = source.replace("{version}", VERSION)
        rc_path = self.directory / f"{crate}-{suffix}.rc"
        rc_path.write_bytes(source.encode("utf-8"))
        output = rc_path.with_suffix(".res")
        command = [str(self.rc), "/nologo"]
        for include in self.includes:
            command.extend(["/I", str(include)])
        if code_page is not None:
            command.append(f"/c{code_page}")
        command.extend(["/fo", str(output), str(rc_path)])
        result = subprocess.run(command, capture_output=True, timeout=30)
        return result, output

    def test_compiles_actual_templates_with_unicode_asset_paths(self):
        for crate in CRATES:
            for code_page in (None, 1252):
                with self.subTest(crate=crate, code_page=code_page):
                    result, output = self.compile_resource(crate, str(code_page), code_page=code_page)
                    self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
                    self.assertTrue(output.is_file())
                    resource = output.read_bytes()
                    manifest = "manifest.manifest" if crate == "wezterm-gui" else "console.manifest"
                    self.assertIn((self.assets / manifest).read_bytes(), resource)
                    self.assertGreater(len(resource), (self.assets / manifest).stat().st_size)
                    if crate == "wezterm-gui":
                        self.assertIn(VERSION.encode("utf-16le"), resource)
                        self.assertGreater(len(resource), (self.assets / "terminal.ico").stat().st_size)

    def test_legacy_code_page_without_pragma_cannot_find_assets(self):
        for crate in CRATES:
            with self.subTest(crate=crate):
                result, output = self.compile_resource(crate, "legacy", pragma=False, code_page=1252)
                self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
                self.assertFalse(output.exists() and output.stat().st_size > 32)


if __name__ == "__main__":
    unittest.main()

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "scripts" / "gx_msvc_env.cmd"
TOOLS = ROOT / ".local" / "tools"
WINDOWS_READY = os.name == "nt" and (TOOLS / "make/bin/make.exe").is_file()


class WindowsEntryPointContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        cls.wrapper = WRAPPER.read_text(encoding="utf-8")

    def test_upstream_check_test_and_build_commands_are_unchanged(self):
        self.assertEqual(
            re.findall(r"^\tcargo check(?: -p ([\w-]+))?$", self.makefile, re.M),
            ["", "wezterm-escape-parser", "wezterm-cell", "wezterm-surface", "wezterm-ssh"],
        )
        self.assertIn("test:\n\tcargo nextest run\n\tcargo nextest run -p wezterm-escape-parser # no_std by default", self.makefile)
        self.assertEqual(
            re.findall(r"^\tcargo build \$\(BUILD_OPTS\) -p ([\w-]+)$", self.makefile, re.M),
            ["wezterm", "wezterm-gui", "wezterm-mux-server", "strip-ansi-escapes"],
        )

    def test_native_make_and_python_layout(self):
        self.assertIn("$(MAKE_HOST)", self.makefile)
        self.assertIn("PATH_SEPARATOR := ;", self.makefile)
        self.assertIn("PATH_SEPARATOR := :", self.makefile)
        self.assertIn("$(wildcard $(TOOLS_ROOT)/venv/Scripts/python.exe)", self.makefile)
        self.assertIn("$(wildcard $(TOOLS_ROOT)/venv/bin/python)", self.makefile)
        self.assertIn("export PATH := $(TOOLS_ROOT)/make/bin", self.makefile)
        self.assertIn("unexport MAKEFLAGS MFLAGS GNUMAKEFLAGS", self.makefile)

    def test_wrapper_uses_discovery_and_project_tools(self):
        self.assertIn("-latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64", self.wrapper)
        self.assertNotIn("Visual Studio\\2022", self.wrapper)
        self.assertIn("%WEZTERM_TOOLCHAIN_ROOT%", self.wrapper)
        self.assertIn('set "PATH=%TOOLS%\\make\\bin;%TOOLS%\\venv\\Scripts;', self.wrapper)
        for variable in ("MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS"):
            self.assertIn(f'set "{variable}="', self.wrapper)

    def test_cache_locations_and_toolchain_are_not_inherited(self):
        for variable, value in (
            ("CARGO_TARGET_DIR", "%REPO%\\target"),
            ("SCCACHE_DIR", "%REPO%\\.local\\sccache"),
            ("TMP", "%REPO%\\.local\\tmp"),
            ("TEMP", "%TMP%"),
            ("TMPDIR", "%TMP%"),
            ("RUSTUP_TOOLCHAIN", "1.96.1-x86_64-pc-windows-msvc"),
        ):
            self.assertIn(f'set "{variable}={value}"', self.wrapper)
            self.assertNotIn(f"if not defined {variable}", self.wrapper)
        self.assertNotIn("target-gx-msvc", self.wrapper)
        self.assertIn("export CARGO_TARGET_DIR := $(CURDIR)/target", self.makefile)
        for variable in ("TMP", "TEMP", "TMPDIR"):
            self.assertIn(f"export {variable} := $(BUILD_TMP_DIR)", self.makefile)

    def test_check_is_read_only_and_does_not_install_toolchains(self):
        self.assertLess(self.wrapper.index('if "%~1"=="--check" goto :check'), self.wrapper.index('mkdir "%TMP%"'))
        check = self.wrapper.split("\n:check\n", 1)[1].split("\n:no_tools\n", 1)[0]
        for command in ("cl /?", "nmake /?", "rc /?", "cmake --version",
                        "rustup run %RUSTUP_TOOLCHAIN% rustc --version",
                        "rustup run %RUSTUP_TOOLCHAIN% cargo --version",
                        "rustup run nightly rustfmt --version"):
            self.assertIn(command, check)
        for forbidden in ("mkdir", " install ", "rustup default", "rustup update"):
            self.assertNotIn(forbidden, check)


@unittest.skipUnless(WINDOWS_READY, "requires Windows and checkout-local setup tools")
class WindowsEntryPointRuntimeTests(unittest.TestCase):
    def run_wrapper(self, *args, env=None):
        result = subprocess.run(
            ["cmd.exe", "/d", "/c", str(WRAPPER), *args], cwd=ROOT,
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        return result.returncode, result.stdout.decode("utf-8", errors="replace")

    def test_check_runs_installed_toolchains(self):
        code, output = self.run_wrapper("--check")
        self.assertEqual(code, 0, output)
        self.assertIn("Rust 1.96.1 and nightly rustfmt", output)

    def test_missing_override_does_not_fall_back_to_system_tools(self):
        env = dict(os.environ, WEZTERM_TOOLCHAIN_ROOT=str(ROOT / ".local" / "missing-gx-tools"))
        code, output = self.run_wrapper("--check", env=env)
        self.assertNotEqual(code, 0, output)
        self.assertIn("missing-gx-tools", output)

    def test_make_exports_native_path_and_keeps_nmake_flags_clean(self):
        temp_root = ROOT / ".local" / "tmp"
        temp_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="gx-msvc-test-", dir=temp_root) as tmp:
            probe = Path(tmp) / "probe.py"
            probe.write_text(
                "import json, os, shutil, sys\n"
                "keys = ['CARGO_TARGET_DIR', 'SCCACHE_DIR', 'TMP', 'TEMP', 'TMPDIR', 'RUSTUP_TOOLCHAIN', 'MAKEFLAGS', 'MFLAGS', 'GNUMAKEFLAGS', 'MAKELEVEL']\n"
                "data = {key: os.environ.get(key) for key in keys}\n"
                "data['PATH'] = os.environ['PATH']\n"
                "data['python'] = sys.executable\n"
                "data['tools'] = {tool: shutil.which(tool) for tool in ['make', 'cargo-nextest', 'stylua', 'lua', 'nasm', 'perl', 'cmake']}\n"
                "print(json.dumps(data))\n", encoding="utf-8",
            )
            makefile = Path(tmp) / "probe.mk"
            makefile.write_text(f'gx-env-probe:\n\t@"$(FRAMEWORK_PY)" "{probe.as_posix()}"\n', encoding="utf-8")
            env = dict(os.environ, WEZTERM_TOOLCHAIN_ROOT=str(TOOLS),
                       CARGO_TARGET_DIR="C:/must-not-use-target", SCCACHE_DIR="C:/must-not-use-cache",
                       TMP="C:/must-not-use-temp", TEMP="C:/must-not-use-temp",
                       MAKEFLAGS="--bad-inherited-option", MFLAGS="--bad-inherited-option",
                       GNUMAKEFLAGS="--bad-inherited-option", RUSTUP_TOOLCHAIN="must-not-use",
                       MAKELEVEL=os.environ.get("MAKELEVEL", "1"))
            code, output = self.run_wrapper("make", "--no-print-directory", "-j2", "-f", "Makefile", "-f", str(makefile), "gx-env-probe", env=env)
            self.assertEqual(code, 0, output)
            data = json.loads(output.strip())
            self.assertEqual(int(data["MAKELEVEL"]), int(env["MAKELEVEL"]) + 1)
            for key, path in (("CARGO_TARGET_DIR", ROOT / "target"),
                              ("SCCACHE_DIR", ROOT / ".local/sccache"),
                              ("TMP", temp_root), ("TEMP", temp_root), ("TMPDIR", temp_root)):
                self.assertEqual(Path(data[key]), path)
            self.assertEqual(data["RUSTUP_TOOLCHAIN"], "1.96.1-x86_64-pc-windows-msvc")
            for key in ("MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS"):
                self.assertFalse(data[key], data)
            self.assertEqual(Path(data["PATH"].split(";")[0]), TOOLS / "make/bin", data)
            self.assertEqual(Path(data["python"]), TOOLS / "venv/Scripts/python.exe")
            for tool, path in data["tools"].items():
                self.assertIsNotNone(path, tool)
                self.assertTrue(Path(path).is_relative_to(TOOLS), (tool, path))


if __name__ == "__main__":
    unittest.main()

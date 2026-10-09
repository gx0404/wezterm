import os
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def find_bash():
    candidates = [shutil.which("bash")]
    if os.name == "nt":
        candidates.insert(0, "C:/Program Files/Git/bin/bash.exe")
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            lowered = candidate.replace("\\", "/").lower()
            if "/windows/system32/" not in lowered and "/windowsapps/" not in lowered:
                return candidate
    return None


@unittest.skipUnless(find_bash(), "Git Bash or POSIX bash is required")
class SetupEnvTests(unittest.TestCase):
    def setUp(self):
        parent = ROOT / ".local/tmp"
        parent.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="test-setup-", dir=parent)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tools = self.root / "tools"
        self.tools.mkdir()

    def script(self, relative, text):
        path = self.tools / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\n" + text + "\n", encoding="utf-8")
        path.chmod(0o755)
        return path

    def version(self, relative, output, code=0):
        return self.script(relative, f"printf '%s\\n' {shlex.quote(output)}; exit {code}")

    def run_shell(self, code, success=True):
        env = os.environ.copy()
        env["SETUP_TEST_TOOLS"] = self.tools.as_posix()
        prelude = """
source scripts/setup_env.sh
TOOLS="$SETUP_TEST_TOOLS"
if command -v cygpath >/dev/null 2>&1; then TOOLS="$(cygpath -u "$TOOLS")"; fi
download() { printf 'unexpected download\\n' >&2; return 91; }
"""
        result = subprocess.run(
            [find_bash(), "--noprofile", "--norc", "-c", prelude + code],
            cwd=ROOT, env=env, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=30,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def healthy_tools(self):
        self.version("nextest/bin/cargo-nextest.exe", "cargo-nextest 0.9.144 (abc)")
        self.version("stylua/bin/stylua.exe", "stylua 2.5.2")
        self.version("lua/bin/lua54.exe", "Lua 5.4.8 Copyright")
        self.version("nasm/bin/nasm.exe", "NASM version 3.02 compiled yesterday")
        self.version("perl/perl/bin/perl.exe", "v5.42.3")
        (self.tools / "perl/.version").write_text("5.42.3.1", encoding="utf-8")
        self.version("perl/c/bin/gmake.exe", "GNU Make 4.4.1")
        for dll in ("libintl-8.dll", "libiconv-2.dll"):
            (self.tools / "perl/c/bin" / dll).write_bytes(b"fixture")
        (self.tools / "perl/licenses").mkdir()
        (self.tools / "perl/licenses/License.rtf").write_text("license fixture", encoding="utf-8")

    def test_exact_versions_and_failed_executables(self):
        cases = [
            ("nextest_ready", "nextest/bin/cargo-nextest.exe", "cargo-nextest 0.9.144", "cargo-nextest 0.9.1440"),
            ("stylua_ready", "stylua/bin/stylua.exe", "stylua 2.5.2", "stylua 2x5x2"),
            ("lua_ready", "lua/bin/lua54.exe", "Lua 5.4.8", "Lua 5.4.80"),
            ("nasm_ready", "nasm/bin/nasm.exe", "NASM version 3.02", "NASM version 3.020"),
            ("make_ready", "make/bin/make.exe", "GNU Make 4.4.1", "GNU Make 4.4.10"),
        ]
        for check, path, good, bad in cases:
            for output, status in ((bad, 0), (good, 1), (good, 0)):
                with self.subTest(check=check, output=output, status=status):
                    self.version(path, output, status)
                    self.run_shell(f'IS_WINDOWS=1; EXE=".exe"; {check}', success=output == good and status == 0)

    def test_perl_requires_package_marker_and_exact_runtime_patch(self):
        self.healthy_tools()
        self.run_shell("perl_ready")
        self.version("perl/perl/bin/perl.exe", "v5.42.2")
        self.run_shell("perl_ready", success=False)
        self.version("perl/perl/bin/perl.exe", "v5.42.3", 1)
        self.run_shell("perl_ready", success=False)
        self.version("perl/perl/bin/perl.exe", "v5.42.3")
        (self.tools / "perl/.version").write_text("5.42.3.0", encoding="utf-8")
        self.run_shell("perl_ready", success=False)

    def test_python_skips_windowsapps_and_old_python_then_uses_next_candidate(self):
        self.script("WindowsApps/python3", 'touch "$TOOLS/alias-called"; exit 0')
        self.script("old/python3", "exit 1")
        self.script("real/python3", 'printf "%s\\n" "$TOOLS/real/python3"')
        result = self.run_shell('''
IS_WINDOWS=0
PATH="$TOOLS/WindowsApps:$TOOLS/old:$TOOLS/real:$PATH"
find_real_python
''')
        self.assertIn("/real/python3", result.stdout)
        self.assertFalse((self.tools / "alias-called").exists())

    def test_python_probe_checks_minimum_version_and_never_executes_alias(self):
        self.script("windowsapps/python", 'touch "$TOOLS/alias-called"')
        self.run_shell('probe_python "$TOOLS/windowsapps/python"', success=False)
        self.assertFalse((self.tools / "alias-called").exists())
        self.script("python", '''
case "$2" in *'sys.version_info < (3, 10)'*) exit 1 ;; esac
printf '%s\\n' "$TOOLS/python"
''')
        self.run_shell('probe_python "$TOOLS/python"', success=False)

    def test_missing_local_tool_does_not_fall_back_to_global_version(self):
        self.version("global/cargo-nextest", "cargo-nextest 0.9.144")
        result = self.run_shell('''
IS_WINDOWS=0; EXE=""
PATH="$TOOLS/global:$PATH"
check_tool nextest_ready nextest
[ "$fail" -eq 1 ]
''')
        self.assertIn("nextest", result.stdout)

    def test_nextest_linux_download_failure_is_explicit_and_no_fallback(self):
        result = self.run_shell('''
IS_WINDOWS=0; EXE=""
download() { printf '%s\\n' "$1" >> "$TOOLS/downloads"; return 1; }
install_nextest
''', success=False)
        self.assertIn("下载失败", result.stderr)
        self.assertNotIn("unbound variable", result.stderr)
        self.assertEqual(len((self.tools / "downloads").read_text().splitlines()), 1)
        self.assertFalse((self.tools / "nextest/bin/cargo-nextest").exists())

    def test_nextest_checksum_failure_never_installs(self):
        result = self.run_shell('''
IS_WINDOWS=0; EXE=""
download() { printf invalid > "$2"; }
install_nextest
''', success=False)
        self.assertIn("sha256", result.stderr)
        self.assertFalse((self.tools / "nextest/bin/cargo-nextest").exists())

    def test_installers_are_idempotent_and_make_retains_dlls_and_license(self):
        self.healthy_tools()
        self.run_shell('''
IS_WINDOWS=1; EXE=".exe"
install_nextest; install_stylua; install_lua; install_nasm; install_perl; install_make
''')
        paths = [p for p in self.tools.rglob("*") if p.is_file()]
        before = {p: (p.read_bytes(), p.stat().st_mtime_ns) for p in paths}
        result = self.run_shell('''
IS_WINDOWS=1; EXE=".exe"
install_nextest; install_stylua; install_lua; install_nasm; install_perl; install_make
''')
        self.assertNotIn("unexpected download", result.stderr)
        self.assertEqual(before, {p: (p.read_bytes(), p.stat().st_mtime_ns) for p in paths})
        for dll in ("libintl-8.dll", "libiconv-2.dll"):
            self.assertEqual((self.tools / "make/bin" / dll).read_bytes(), b"fixture")
        self.assertTrue((self.tools / "perl/licenses/License.rtf").is_file())

    def test_make_rejects_wrong_bundled_version(self):
        self.healthy_tools()
        self.version("perl/c/bin/gmake.exe", "GNU Make 4.4.10")
        self.run_shell("IS_WINDOWS=1; install_make", success=False)
        self.assertFalse((self.tools / "make/bin/make.exe").exists())

    def test_venv_repairs_python3_without_reinstalling_packages(self):
        self.version("venv/Scripts/python.exe", "Python 3.12.0")
        self.version("venv/Scripts/graphify.exe", "graphify 0.9.73")
        self.run_shell('''
IS_WINDOWS=1
venv_packages_ready() { return 0; }
venv_ready() { [ -f "$TOOLS/venv/Scripts/python3.exe" ] && [ -x "$TOOLS/venv/bin/graphify" ]; }
install_venv
install_venv
''')
        self.assertEqual((self.tools / "venv/Scripts/python.exe").read_bytes(),
                         (self.tools / "venv/Scripts/python3.exe").read_bytes())

    def test_check_mode_is_read_only_and_main_propagates_health_failure(self):
        before = list(self.tools.rglob("*"))
        self.run_shell('MODE=check; check_all() { return 7; }; main', success=False)
        self.assertEqual(before, list(self.tools.rglob("*")))
        result = self.run_shell('''
ROOT="$TOOLS"
need_cmd() { :; }; need_python3() { :; }
install_nextest() { :; }; install_venv() { :; }; install_stylua() { :; }
install_nasm() { :; }; install_perl() { :; }; install_make() { :; }; install_lua() { :; }
check_all() { return 7; }
main
''', success=False)
        self.assertEqual(result.returncode, 7)
        self.assertNotIn("完成", result.stdout)

    def test_wrong_installed_version_is_not_an_idempotent_success(self):
        self.version("nextest/bin/cargo-nextest.exe", "cargo-nextest 0.9.1440")
        result = self.run_shell('IS_WINDOWS=1; EXE=".exe"; install_nextest', success=False)
        self.assertIn("unexpected download", result.stderr)
        self.assertNotIn("跳过", result.stdout)

    def test_venv_wrong_package_version_does_not_recreate_its_own_interpreter(self):
        self.script("venv/Scripts/python.exe", 'printf "%s\\n" "$TOOLS/venv/Scripts/python.exe"')
        self.version("venv/Scripts/graphify.exe", "graphify 0.9.73")
        self.run_shell('''
IS_WINDOWS=1
venv_packages_ready() { return 1; }
venv_ready() { [ -f "$TOOLS/packages-repaired" ]; }
uv() {
    [ "$1" = pip ] || return 90
    [ "$2" = install ] || return 91
    [ "$5" = "graphifyy==0.9.73" ] || return 92
    touch "$TOOLS/packages-repaired"
}
install_venv
''')


if __name__ == "__main__":
    unittest.main()

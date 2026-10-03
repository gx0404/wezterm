"""dev_framework 在 Windows 上经 bash 调度 .sh 入口的单元测试；不依赖本机 bash、网络或真实 Windows。"""
import contextlib
import io
import os
import tempfile
import unittest
from pathlib import Path, PureWindowsPath
from types import SimpleNamespace
from unittest.mock import patch

import dev_framework as framework


class FakeOs:
    """只替换被测模块看到的 os.name 与 os.pathsep，其余属性转发真实 os。

    不改全局 os.name：否则 pathlib.Path 会按被伪装的平台构造路径（Linux 上
    3.10/3.11 直接抛错），临时目录里的假 bash.exe 就查不到了。
    """

    def __init__(self, name, pathsep):
        self.name, self.pathsep = name, pathsep

    def __getattr__(self, attr):
        return getattr(os, attr)


WINDOWS = FakeOs("nt", ";")
POSIX = FakeOs("posix", ":")


class WindowsShellDispatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git = self.install("Git/usr/bin")
        self.msys = self.install("msys64/usr/bin")
        self.empty = self.root / "empty"
        self.empty.mkdir()
        # 大小写敏感的文件系统上每个变体是独立目录、各放一份 bash.exe，
        # 保证用例验证的是「跳过」而不是「文件不存在」。
        self.system32 = [str(self.install("Windows/System32")),
                         str(self.install("WINDOWS/SYSTEM32")) + os.sep,
                         str(self.install("windows/system32")) + "/"]
        self.windows_apps = [str(self.install("Local/Microsoft/WindowsApps")),
                             str(self.install("Local/Microsoft/windowsapps")) + "/",
                             str(self.install("Program Files/WINDOWSAPPS/Pkg_1.0")) + os.sep]

    def install(self, relative):
        folder = self.root / relative
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "bash.exe").write_bytes(b"MZ")
        return folder

    @contextlib.contextmanager
    def simulate(self, platform, *entries, system_root=None):
        env = {"PATH": platform.pathsep.join(entries),
               "SYSTEMROOT": str(self.root / "Windows") if system_root is None else system_root}
        with patch.object(framework, "os", platform), patch.dict(os.environ, env):
            yield

    def test_system32_launcher_is_skipped_in_any_case_or_trailing_separator(self):
        for system_root in (str(self.root / "Windows"), str(self.root / "WINDOWS") + os.sep):
            for entry in self.system32:
                with self.subTest(system_root=system_root, entry=entry), \
                        self.simulate(WINDOWS, entry, str(self.git), system_root=system_root):
                    self.assertEqual(framework.windows_bash(), str(self.git / "bash.exe"))

    def test_windows_apps_launchers_are_skipped_in_any_case_or_depth(self):
        for entry in self.windows_apps:
            with self.subTest(entry=entry), self.simulate(WINDOWS, entry, str(self.git)):
                self.assertEqual(framework.windows_bash(), str(self.git / "bash.exe"))

    def test_literal_system32_is_skipped_with_real_empty_or_missing_system_root(self):
        launcher = PureWindowsPath(r"C:\Windows\System32\bash.exe")
        real_is_file = Path.is_file

        def is_file(path):
            # 假定默认位置装有 WSL 启动器，不依赖本机实际安装与宿主路径语义。
            return PureWindowsPath(str(path)) == launcher or real_is_file(path)

        entries = (r"C:\WINDOWS\system32", "c:\\windows\\SYSTEM32\\", "C:/Windows/System32/", str(self.git))
        for system_root in (r"C:\WINDOWS", "", None):
            with self.subTest(system_root=system_root), self.simulate(WINDOWS, *entries), \
                    patch.object(Path, "is_file", is_file):
                if system_root is None:
                    del os.environ["SYSTEMROOT"]
                else:
                    os.environ["SYSTEMROOT"] = system_root
                self.assertEqual(framework.windows_bash(), str(self.git / "bash.exe"))

    def test_first_usable_path_entry_wins(self):
        for entries, expected in (((str(self.msys), str(self.git)), self.msys),
                                  ((str(self.git), str(self.msys)), self.git),
                                  ((str(self.empty), self.git.as_posix() + "/"), self.git)):
            with self.subTest(entries=entries), self.simulate(WINDOWS, *entries):
                self.assertEqual(framework.windows_bash(), str(expected / "bash.exe"))

    def test_path_without_usable_bash_raises(self):
        # 当前目录放着 bash.exe：空 PATH 项或缺失的 PATH 若被当成当前目录就会误命中。
        self.addCleanup(os.chdir, os.getcwd())
        os.chdir(self.git)
        for entries in ((), ("", str(self.empty), *self.system32, *self.windows_apps)):
            with self.subTest(entries=entries), self.simulate(WINDOWS, *entries):
                with self.assertRaisesRegex(ValueError, r"bash\.exe"):
                    framework.windows_bash()
                with self.assertRaisesRegex(ValueError, r"bash\.exe"):
                    framework.command(["scripts/setup_env.sh"])
        with self.simulate(WINDOWS):
            del os.environ["PATH"]
            with self.assertRaisesRegex(ValueError, r"bash\.exe"):
                framework.windows_bash()

    def test_command_keeps_sh_argv_outside_windows(self):
        with self.simulate(POSIX):
            self.assertEqual(framework.command(["scripts/graphify.sh", "check"]), ["scripts/graphify.sh", "check"])

    def test_command_prepends_resolved_bash_to_windows_sh_entry(self):
        argv = ["scripts/graphify.sh", "check"]
        with self.simulate(WINDOWS, self.system32[0], self.windows_apps[0], str(self.git)):
            self.assertEqual(framework.command(argv), [str(self.git / "bash.exe"), "scripts/graphify.sh", "check"])
        self.assertEqual(argv, ["scripts/graphify.sh", "check"])

    def test_command_keeps_non_sh_argv_on_windows_without_bash(self):
        for argv in (["python3", "scripts/build_agent_kb.py", "--check"], ["make", "check"],
                     ["bash", "scripts/setup_env.sh"]):
            with self.subTest(argv=argv), self.simulate(WINDOWS, str(self.empty)):
                self.assertEqual(framework.command(list(argv)), argv)

    def test_run_spawns_windows_sh_entry_through_bash_or_not_at_all(self):
        data = {"commands": {"graph-check": {"status": "configured", "argv": ["scripts/graphify.sh", "check"],
                                             "cwd": "."}}}
        with self.simulate(WINDOWS, str(self.git)), \
                patch.object(framework.subprocess, "run", return_value=SimpleNamespace(returncode=3)) as spawn:
            self.assertEqual(framework.run(data, "graph-check"), 3)
        spawn.assert_called_once_with([str(self.git / "bash.exe"), "scripts/graphify.sh", "check"],
                                      cwd=framework.ROOT, check=False)
        with self.simulate(WINDOWS, str(self.empty)), patch.object(framework.subprocess, "run") as spawn:
            with self.assertRaisesRegex(ValueError, r"bash\.exe"):
                framework.run(data, "graph-check")
        spawn.assert_not_called()

    def doctor(self, platform, *entries):
        # 用仓库里带可执行位的真实入口（临时目录可能挂 noexec），MISSING 只能来自 bash 解析。
        data = {"commands": {"setup": {"status": "configured", "argv": ["scripts/setup_env.sh"], "cwd": "."}}}
        output = io.StringIO()
        with self.simulate(platform, *entries), patch.object(framework, "config", return_value=data), \
                patch.object(framework.sys, "argv", ["dev_framework.py", "doctor"]), \
                contextlib.redirect_stdout(output):
            code = framework.main()
        return code, output.getvalue().splitlines()[0]

    def test_doctor_reports_windows_sh_entry_missing_without_bash(self):
        code, line = self.doctor(WINDOWS, str(self.empty), *self.system32, *self.windows_apps)
        self.assertEqual(code, 1)
        self.assertRegex(line, r"^MISSING setup: .*bash\.exe.*；未运行$")

    def test_doctor_finds_sh_entry_with_windows_bash_or_outside_windows(self):
        for platform, entries in ((WINDOWS, (str(self.git),)), (POSIX, ())):
            with self.subTest(platform=platform.name):
                self.assertEqual(self.doctor(platform, *entries), (0, "FOUND setup: 可执行文件；未运行"))


class UiSmokeDispatchTests(unittest.TestCase):
    """ui-smoke 在 Windows 上换成 PowerShell 入口（Xvfb/xwd/ffmpeg 版 .sh 在 Windows 不可用），其余平台不变。"""

    OUT = ".ui-evidence/feature%2Fx/ui/20260101T000000Z-abcd1234"
    DATA = {"evidence_root": ".ui-evidence",
            "commands": {"ui-smoke": {"status": "configured", "argv": ["scripts/ui_smoke.sh"], "cwd": "."},
                         "graph-check": {"status": "configured", "argv": ["scripts/graphify.sh", "check"],
                                         "cwd": "."}}}

    @contextlib.contextmanager
    def simulate(self, platform, found=()):
        """found 是 PATH 上能找到的命令名到路径的映射；其余查找一律返回 None。"""
        with patch.object(framework, "os", platform), \
                patch.object(framework.shutil, "which", side_effect=lambda name: dict(found).get(name)):
            yield

    def test_posix_keeps_shell_entry_and_never_looks_for_powershell(self):
        with patch.object(framework, "os", POSIX), \
                patch.object(framework.shutil, "which", side_effect=AssertionError("不应查找 PowerShell")):
            self.assertEqual(framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"]),
                             ["scripts/ui_smoke.sh"])
            self.assertEqual(framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"], self.OUT),
                             ["scripts/ui_smoke.sh", "--out", self.OUT])

    def test_windows_runs_powershell_script_with_out(self):
        with self.simulate(WINDOWS, {"pwsh": r"C:\pwsh\pwsh.exe", "powershell": r"C:\ps\powershell.exe"}):
            self.assertEqual(framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"], self.OUT),
                             [r"C:\pwsh\pwsh.exe", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
                              "-File", "scripts/ui_smoke_windows.ps1", "-Out", self.OUT])

    def test_windows_falls_back_to_powershell_then_fails_without_either(self):
        with self.simulate(WINDOWS, {"powershell": r"C:\ps\powershell.exe"}):
            self.assertEqual(framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"], self.OUT)[0],
                             r"C:\ps\powershell.exe")
        with self.simulate(WINDOWS), self.assertRaisesRegex(ValueError, "pwsh"):
            framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"], self.OUT)

    def test_windows_default_out_follows_smoke_directory_convention(self):
        with self.simulate(WINDOWS, {"pwsh": r"C:\pwsh\pwsh.exe"}):
            argv = framework.entry(self.DATA, "ui-smoke", ["scripts/ui_smoke.sh"])
        self.assertEqual(argv[-2], "-Out")
        self.assertRegex(argv[argv.index("-Out") + 1], r"^\.ui-evidence/smoke/\d{8}T\d{6}Z-\d+$")

    def test_windows_leaves_other_targets_on_the_bash_path(self):
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "bash.exe").write_bytes(b"MZ")
            with self.simulate(WINDOWS, {"pwsh": r"C:\pwsh\pwsh.exe"}), \
                    patch.dict(os.environ, {"PATH": folder, "SYSTEMROOT": r"C:\Windows"}):
                self.assertEqual(framework.entry(self.DATA, "graph-check", ["scripts/graphify.sh", "check"]),
                                 [str(Path(folder) / "bash.exe"), "scripts/graphify.sh", "check"])

    def test_run_spawns_the_resolved_entry_from_the_repo_root(self):
        with self.simulate(WINDOWS, {"pwsh": r"C:\pwsh\pwsh.exe"}), \
                patch.object(framework.subprocess, "run", return_value=SimpleNamespace(returncode=5)) as spawn:
            self.assertEqual(framework.run(self.DATA, "ui-smoke", self.OUT), 5)
        spawn.assert_called_once_with(
            [r"C:\pwsh\pwsh.exe", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
             "-File", "scripts/ui_smoke_windows.ps1", "-Out", self.OUT], cwd=framework.ROOT, check=False)

    def test_run_rejects_out_for_other_targets_or_outside_evidence_root(self):
        with patch.object(framework.subprocess, "run") as spawn:
            for target, out in (("graph-check", self.OUT), ("ui-smoke", "docs/evidence"),
                                ("ui-smoke", ".ui-evidence-evil/x"), ("ui-smoke", ".ui-evidence/../x")):
                with self.subTest(target=target, out=out), self.assertRaises(ValueError):
                    framework.run(self.DATA, target, out)
        spawn.assert_not_called()

    def doctor(self, platform, found=()):
        output = io.StringIO()
        with self.simulate(platform, found), patch.object(framework, "config", return_value=self.DATA), \
                patch.object(framework.sys, "argv", ["dev_framework.py", "doctor"]), \
                contextlib.redirect_stdout(output):
            code = framework.main()
        return code, [line for line in output.getvalue().splitlines() if "ui-smoke" in line]

    def test_doctor_checks_the_windows_script_and_shell_not_bash(self):
        # PATH 上没有 bash：.sh 入口会 MISSING，而 ui-smoke 走 PowerShell 入口应 FOUND。
        code, lines = self.doctor(WINDOWS, {"pwsh": r"C:\pwsh\pwsh.exe"})
        self.assertEqual(lines, ["FOUND ui-smoke: 可执行文件；未运行"])
        code, lines = self.doctor(WINDOWS)
        self.assertEqual(code, 1)
        self.assertRegex(lines[0], r"^MISSING ui-smoke: .*pwsh.*；未运行$")


if __name__ == "__main__":
    unittest.main()

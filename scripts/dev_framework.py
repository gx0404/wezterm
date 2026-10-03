#!/usr/bin/env python3
"""开发命令与证据目录入口；配置完整与实际运行分别验证。"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath, PureWindowsPath
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
CONFIG = "docs/dev-framework.json"


def within(raw: str, *, allow_root: bool = False) -> Path:
    if not isinstance(raw, str) or not raw or "\\" in raw:
        raise ValueError("路径必须是仓库相对路径")
    parts = PurePosixPath(raw)
    if parts.is_absolute() or ".." in parts.parts or (raw == "." and not allow_root):
        raise ValueError("路径不得越出仓库")
    target = ROOT / raw
    current = target
    while current != ROOT:
        if current.is_symlink():
            raise ValueError("不允许经符号链接访问框架文件或证据")
        current = current.parent
    if not target.resolve().is_relative_to(ROOT):
        raise ValueError("路径解析后越出仓库")
    return target


def config() -> dict:
    data = json.loads(within(CONFIG).read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema") != 1 or not isinstance(data.get("commands"), dict):
        raise ValueError("开发框架配置版本或 commands 无效")
    if not isinstance(data.get("project"), dict):
        raise ValueError("project 必须是对象")
    evidence_root = data.get("evidence_root")
    if (not isinstance(evidence_root, str) or not re.fullmatch(r"\.[a-zA-Z0-9_-]+", evidence_root)
            or evidence_root in {".git", ".claude", ".codex", ".agents", ".kimi-code", ".zcode", ".venv"}):
        raise ValueError("evidence_root 必须是独立的隐藏证据目录，不能指向业务或工具目录")
    if not isinstance(data.get("ci"), list) or not data["ci"]:
        raise ValueError("ci 必须声明检查入口")
    for target in data["ci"]:
        if not isinstance(target, str) or target in {"setup", "dev", "build", "package", "graph", "kb"}:
            raise ValueError("CI 不可包含安装、服务启动、生成写入或打包目标")
        if target not in data["commands"]:
            raise ValueError(f"CI 引用了未登记目标：{target}")
    for name, item in data["commands"].items():
        if not re.fullmatch(r"[a-z][a-z0-9-]*", name) or not isinstance(item, dict):
            raise ValueError("命令名或声明无效")
        state = item.get("status")
        if state not in {"configured", "pending", "not-applicable"}:
            raise ValueError(f"{name} 缺少有效状态")
        if state == "configured":
            argv = item.get("argv")
            if not isinstance(argv, list) or not argv or any(not isinstance(x, str) or not x or "\0" in x for x in argv):
                raise ValueError(f"{name} 必须使用非空 argv 数组")
            if not within(item.get("cwd", "."), allow_root=True).is_dir():
                raise ValueError(f"{name} 工作目录不存在")
        elif not isinstance(item.get("reason"), str) or not item["reason"].strip():
            raise ValueError(f"{name} 必须解释 pending / N/A 的原因")
        if name in {"lint", "test"} and state == "not-applicable":
            raise ValueError(f"{name} 是基本质量门，不能 N/A")
    for name in ("lint", "test"):
        if name not in data["commands"] or name not in data["ci"]:
            raise ValueError(f"缺少基础质量门：{name}")
    within(data["evidence_root"])
    return data


def ready(data: dict) -> None:
    pending = [name for name, item in data["commands"].items() if item["status"] == "pending"]
    if data.get("project", {}).get("reviewed") is not True:
        pending.append("项目轮廓与领域规则尚未核实")
    if pending:
        raise ValueError("PENDING: " + ", ".join(pending))


def windows_bash() -> str:
    # System32 与 WindowsApps 下的 bash.exe 是 WSL 启动器，不能解释仓库脚本。
    system32 = PureWindowsPath(os.environ.get("SYSTEMROOT") or r"C:\Windows", "System32")
    for entry in os.environ.get("PATH", "").split(os.pathsep):
        folder = PureWindowsPath(entry)
        if not entry or folder == system32 or "windowsapps" in (part.lower() for part in folder.parts):
            continue
        candidate = Path(entry) / "bash.exe"
        if candidate.is_file():
            return str(candidate)
    raise ValueError("Windows 上运行 .sh 需要 PATH 中有 Git Bash 或 MSYS2 的 bash.exe"
                     "（System32、WindowsApps 下的 WSL 启动器不可用）")


def command(argv: list[str]) -> list[str]:
    # Windows 不能直接执行 .sh（WinError 193），改由 bash 解释。
    if os.name == "nt" and argv[0].endswith(".sh"):
        return [windows_bash(), *argv]
    return argv


# 个别命令在 Windows 上换成专用入口：原 .sh 依赖 Xvfb/xwd/ffmpeg，Windows 没有。
WINDOWS_ENTRIES = {"ui-smoke": "scripts/ui_smoke_windows.ps1"}


def windows_powershell() -> str:
    for name in ("pwsh", "powershell"):
        found = shutil.which(name)
        if found:
            return found
    raise ValueError("Windows 上运行 ui-smoke 需要 PATH 中有 pwsh 或 powershell")


def entry(data: dict, target: str, argv: list[str], out: str | None = None) -> list[str]:
    """命令名换成实际 argv：ui-smoke 在 Windows 上走 PowerShell 入口，其余沿用 command()。

    out 是批次目录（仓库相对路径），只对 ui-smoke 透传；缺省时 Windows 入口按
    ui_smoke.sh 的约定落到 <evidence_root>/smoke/<UTC 时间戳>-<pid>。
    """
    script = WINDOWS_ENTRIES.get(target) if os.name == "nt" else None
    if script:
        if out is None:
            stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
            out = f"{data['evidence_root']}/smoke/{stamp}-{os.getpid()}"
        return [windows_powershell(), "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
                "-File", script, "-Out", out]
    argv = command(argv)
    if target == "ui-smoke" and out is not None:
        argv = [*argv, "--out", out]
    return argv


def run(data: dict, target: str, out: str | None = None) -> int:
    if target not in data["commands"]:
        raise ValueError(f"未知命令：{target}")
    item = data["commands"][target]
    if item["status"] == "pending":
        raise ValueError(f"PENDING {target}: {item['reason']}")
    if item["status"] == "not-applicable":
        print(f"N/A {target}: {item['reason']}")
        return 0
    if out is not None:
        root = data["evidence_root"]
        if target != "ui-smoke" or not (out == root or out.startswith(root + "/")):
            raise ValueError(f"--out 只用于 ui-smoke，且必须是 {root}/ 下的批次目录")
        within(out)
    # 仅显式 run/ci 执行已审阅命令；不使用 shell 拼接和自动重试。
    return subprocess.run(entry(data, target, item["argv"], out), cwd=within(item.get("cwd", "."), allow_root=True),
                          check=False).returncode


def branch() -> str:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    def read(*args: str) -> str:
        result = subprocess.run(["git", "-C", str(ROOT), *args], env=env,
                                capture_output=True, text=True, timeout=3, check=False)
        return result.stdout.strip() if result.returncode == 0 else ""
    try:
        name = read("branch", "--show-current")
        if name:
            return name
        commit = read("rev-parse", "--short", "HEAD")
        return "detached-" + commit if commit else "no-git"
    except (OSError, subprocess.TimeoutExpired):
        return "no-git"


def component(value: str) -> str:
    # 保留中文；百分号与斜杠编码，避免两个不同分支共用目录。
    text = "".join(ch if ch.isalnum() or ch in "_.-" else quote(ch, safe="") for ch in value)
    if text in {".", ".."}:
        text = text.replace(".", "%2E")
    if len(text.encode()) > 160:
        text = text.encode()[:140].decode(errors="ignore") + "--" + hashlib.sha256(value.encode()).hexdigest()[:12]
    return text or "no-git"


def evidence(data: dict, task: str) -> None:
    if not re.fullmatch(r"[^\W_][\w.-]{0,100}", task):
        raise ValueError("任务名仅限文字、数字、点、下划线和横线")
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    branch_name = branch()
    relative = f"{data['evidence_root']}/{component(branch_name)}/{task}/{run_id}"
    folder = within(relative)
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    try:
        result = subprocess.run(["git", "-C", str(ROOT), "check-ignore", "--no-index", "--quiet", relative + "/result.json"],
                                env=env, capture_output=True, timeout=3, check=False)
        if result.returncode == 1:
            raise ValueError("证据目录尚未被 Git 忽略，拒绝写入")
        if result.returncode not in {0, 128}:
            raise ValueError("无法确定证据目录的 Git 忽略状态")
    except FileNotFoundError:
        pass  # 无 Git 的候选目录仍可分配证据，不能据此声称跟踪边界已验证。
    folder.mkdir(parents=True, exist_ok=False)
    for name in ("results", "report"):
        (folder / name).mkdir()
    (folder / "result.json").write_text(json.dumps({"status": "PENDING", "task": task,
        "branch": branch_name, "run": run_id, "screenshots": [], "images_reviewed": False,
        "note": "只分配证据目录；尚未运行 UI、截图或读图"}, ensure_ascii=False, indent=2) + "\n")
    print(relative)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["help", "check", "ready", "run", "ci", "doctor", "evidence"])
    parser.add_argument("target", nargs="?")
    parser.add_argument("--out", help="run ui-smoke：批次目录（evidence_root 下的仓库相对路径）")
    args = parser.parse_args()
    try:
        data = config()
        if args.out is not None and args.action != "run":
            raise ValueError("--out 只能配合 run ui-smoke 使用")
        if args.action == "help":
            print("make framework-check / framework-ready / ai-doctor / ci-check")
            print("make version / version-check / version-write / evidence")
            for name, item in data["commands"].items():
                print(f"make {name}: {item['status']}")
            return 0
        elif args.action == "check":
            # fork: 规则闭集之外再守 dotfiles 漂移（WEZ-CFG-02）：
            # gx-sync --check 对本机配置/插件与仓库快照的差异返回非零。
            for argv in ([sys.executable, str(ROOT / "scripts/resolve_agent_rules.py"), "--check"],
                         [sys.executable, str(ROOT / "scripts/gx_bundle.py"), "sync", "--check"]):
                result = subprocess.run(argv, check=False)
                if result.returncode:
                    return result.returncode
            return 0
        if args.action == "ready":
            ready(data)
            print("PASS 配置完整；实际命令结果须另行验证")
        elif args.action == "run":
            return run(data, args.target or "", args.out)
        elif args.action == "ci":
            ready(data)
            for argv in ([sys.executable, str(ROOT / "scripts/resolve_agent_rules.py"), "--check"],
                         [sys.executable, str(ROOT / "scripts/version.py"), "--check"]):
                result = subprocess.run(argv, check=False)
                if result.returncode:
                    return result.returncode
            for target in data["ci"]:
                code = run(data, target)
                if code:
                    return code
        elif args.action == "evidence":
            evidence(data, args.target or "")
        else:
            failed = False
            for name, item in data["commands"].items():
                state = item["status"]
                if state == "configured":
                    cwd = within(item.get("cwd", "."), allow_root=True)
                    windows_script = WINDOWS_ENTRIES.get(name) if os.name == "nt" else None
                    executable = windows_script or item["argv"][0]
                    if windows_script:
                        found = (cwd / windows_script).is_file()  # .ps1 靠解释器执行，无可执行位
                    else:
                        found = os.access(cwd / executable, os.X_OK) if "/" in executable else bool(shutil.which(executable))
                    detail = "可执行文件；未运行"
                    try:
                        entry(data, name, item["argv"])  # 与 run 同一解析：Windows 的 .sh 入口还须找到 bash
                    except ValueError as exc:
                        found, detail = False, f"{exc}；未运行"
                    print(f"{'FOUND' if found else 'MISSING'} {name}: {detail}")
                    failed |= not found
                else:
                    print(f"{state.upper()} {name}: {item['reason']}")
            print("INFO 此诊断只检查命令入口；已选客户端/MCP/读图能力按 AI_TOOLS 分项验证，未启用项记 N/A。")
            return int(failed)
        return 0
    except (ValueError, KeyError, TypeError, OSError, subprocess.TimeoutExpired) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

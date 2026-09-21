#!/usr/bin/env python3
"""hooks 安全门探针：用无副作用 JSON 输入验证允许/拒绝语义。

除直调引擎外，还按各工具配置里的**原样注册方式**走注册入口（含适配器与
解释器），并做适配器语言完整性（python3 调用的必须是真 Python）与
codex/zcode 配置形状锁——防止 shell 冒充 .py、Claude 风格单表 hooks 等
"直调探针全绿"的入口层错误（2026-09 wezterm 落地事故沉淀）。
"""

from __future__ import annotations

import ast
import importlib.util
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

REPO_ROOT = Path(__file__).resolve().parents[1]
GATE = REPO_ROOT / ".claude" / "hooks" / "pre_tool_use_gate.py"
ADAPTER = REPO_ROOT / ".claude" / "hooks" / "block_dangerous.sh"
CODEX_ADAPTER = REPO_ROOT / ".codex" / "hooks" / "pre_tool_use_policy.py"
ROOT_TOKEN = "$(git rev-parse --show-toplevel)"


def _load_engine():
    spec = importlib.util.spec_from_file_location("pre_tool_use_gate", GATE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


class EngineSemantics(unittest.TestCase):
    def setUp(self) -> None:
        self.engine = _load_engine()
        self.patterns = self.engine.load_patterns()
        self.root = REPO_ROOT

    def _eval(self, tool: str, tool_input: dict):
        return self.engine.evaluate(tool, tool_input, self.patterns, repo_root=self.root)

    def test_allow_plain_git_status(self) -> None:
        self.assertEqual(self._eval("Bash", {"command": "git status"}), (None, None))

    def test_deny_force_push(self) -> None:
        level, _ = self._eval("Bash", {"command": "git push --force origin main"})
        self.assertEqual(level, "deny")

    def test_ask_force_with_lease(self) -> None:
        level, _ = self._eval("Bash", {"command": "git push --force-with-lease origin main"})
        self.assertEqual(level, "ask")

    def test_deny_push_upstream(self) -> None:
        level, _ = self._eval("Bash", {"command": "git push upstream feature/x"})
        self.assertEqual(level, "deny")

    def test_deny_no_verify_commit(self) -> None:
        level, _ = self._eval("Bash", {"command": "git commit --no-verify -m x"})
        self.assertEqual(level, "deny")

    def test_deny_cargo_publish(self) -> None:
        level, _ = self._eval("Bash", {"command": "cargo publish"})
        self.assertEqual(level, "deny")

    def test_ask_git_add_all(self) -> None:
        level, _ = self._eval("Bash", {"command": "git add -A"})
        self.assertEqual(level, "ask")

    def test_ask_matches_command_position_only(self) -> None:
        # ask 级模式锚定命令位置：真执行（含链式/包装/环境变量前缀）升级，
        # 文本提及（heredoc 正文、搜索关键字、commit message）不命中。
        for command in (
            "pkill -9 wezterm",
            "cargo build && pkill wezterm-gui",
            "sudo killall wezterm",
            "pgrep wezterm | xargs -r pkill -f",
            "bash -c 'pkill wezterm'",
            "bash <<'EOF'\npkill wezterm\nEOF",
            "WEZTERM_X=1 pkill wezterm",
            "cd /tmp && git add -A",
            "if true; then git reset --hard HEAD~1; fi",
            "git clean -fd",
        ):
            level, _ = self._eval("Bash", {"command": command})
            self.assertEqual(level, "ask", command)
        for command in (
            "rg pkill docs/",
            "python3 - <<'PYEOF'\ntext = '禁 pkill/猜 PID'\nprint(len(text))\nPYEOF",
            "python3 - <<'PYEOF'\ntext = '禁 git add -A、git reset --hard 与 git clean'\nPYEOF",
            "git commit -m 'docs: 说明为何不用 git add --all'",
        ):
            self.assertEqual(self._eval("Bash", {"command": command}), (None, None), command)

    def test_deny_matches_command_position_only(self) -> None:
        # deny 级同样锚定命令位置；重定向类模式（> 路径）无命令位置，不在此列。
        for command in (
            "make test && git push origin main --force",
            "git -C /tmp/x push -f origin main",
            "git --no-pager push upstream feature/x",
            "bash -c 'cargo publish'",
            "GH_TOKEN=x gh release create v9.9.9",
            "bash <<'EOF'\ngit filter-branch --all\nEOF",
            "echo x | sudo tee docs/changelog.md",
            "if true; then git commit -m x --no-verify; fi",
        ):
            level, _ = self._eval("Bash", {"command": command})
            self.assertEqual(level, "deny", command)
        for command in (
            "rg 'cargo publish' docs/",
            "python3 - <<'PYEOF'\ntext = '禁 git push --force、git push upstream 与 cargo publish'\nPYEOF",
            "python3 - <<'PYEOF'\ntext = '勿用 sed 改 docs/changelog.md，勿 cat ~/.ssh/id_rsa'\nPYEOF",
            "git commit -m 'docs: 解释为何禁止 git push --force 与 gh release create'",
        ):
            self.assertEqual(self._eval("Bash", {"command": command}), (None, None), command)

    def test_deny_read_credentials_via_cat(self) -> None:
        level, _ = self._eval("Bash", {"command": "cat ~/.ssh/id_rsa"})
        self.assertEqual(level, "deny")

    def test_file_deny_generated_graph(self) -> None:
        level, _ = self._eval("Write", {"file_path": str(REPO_ROOT / "graphify-out" / "graph.json")})
        self.assertEqual(level, "deny")

    def test_file_deny_upstream_changelog_absolute_path(self) -> None:
        level, _ = self._eval("Edit", {"file_path": str(REPO_ROOT / "docs" / "changelog.md")})
        self.assertEqual(level, "deny")

    def test_file_allow_normal_source(self) -> None:
        level, _ = self._eval("Edit", {"file_path": str(REPO_ROOT / "term" / "src" / "lib.rs")})
        self.assertIsNone(level)

    def test_file_deny_env(self) -> None:
        level, _ = self._eval("Write", {"file_path": ".env"})
        self.assertEqual(level, "deny")

    def test_allow_typical_workflow_commands(self) -> None:
        for command in (
            "python3 scripts/resolve_agent_rules.py term",
            "make framework-check",
            "cargo nextest run -p term",
            "git diff --stat",
        ):
            self.assertEqual(self._eval("Bash", {"command": command}), (None, None), command)


class AdapterProtocol(unittest.TestCase):
    def _run(self, payload: dict, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(ADAPTER), *extra],
            input=json.dumps(payload), capture_output=True, text=True, timeout=30,
        )

    def test_adapter_denies_force_push_with_json_decision(self) -> None:
        result = self._run({"tool_name": "Bash", "tool_input": {"command": "git push -f origin x"}})
        self.assertEqual(result.returncode, 0)
        decision = json.loads(result.stdout)
        self.assertEqual(
            decision["hookSpecificOutput"]["permissionDecision"], "deny",
        )

    def test_adapter_allows_git_status_silently(self) -> None:
        result = self._run({"tool_name": "Bash", "tool_input": {"command": "git status"}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "")

    def test_codex_protocol_payload_shape(self) -> None:
        result = subprocess.run(
            [sys.executable, str(GATE), "--protocol", "codex"],
            input=json.dumps({"tool_name": "Bash", "tool_input": {"command": "git push -f origin x"}}),
            capture_output=True, text=True, timeout=30,
        )
        decision = json.loads(result.stdout)
        self.assertEqual(decision["permissionDecision"], "deny")
        self.assertIn("reason", decision)


class RegisteredEntryProbes(unittest.TestCase):
    """按工具配置里的原样注册方式走一遍入口（解释器 + 适配器路径）。"""

    def test_codex_adapter_via_registered_python_invocation(self) -> None:
        # 危险字面量拆分构造：宿主会话可能挂着同一安全门，命令文本不得整串出现。
        deny_command = "git pu" + "sh --force origin main"
        result = subprocess.run(
            [sys.executable, str(CODEX_ADAPTER)],
            input=json.dumps({"tool_name": "Bash", "tool_input": {"command": deny_command}}),
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        decision = json.loads(result.stdout)
        self.assertEqual(decision["permissionDecision"], "deny")

    def test_codex_adapter_allows_normal_command(self) -> None:
        result = subprocess.run(
            [sys.executable, str(CODEX_ADAPTER)],
            input=json.dumps({"tool_name": "Bash", "tool_input": {"command": "make framework-check"}}),
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "")


def _registered_commands() -> list[tuple[str, str]]:
    """收集三份工具配置里登记的全部 hook 命令（已把 $(git rev-parse...) 归一到仓库根）。"""
    commands: list[tuple[str, str]] = []
    claude = json.loads((REPO_ROOT / ".claude" / "settings.json").read_text(encoding="utf-8"))
    for block in claude.get("hooks", {}).get("PreToolUse", []):
        for hook in block.get("hooks", []):
            commands.append(("claude", hook["command"]))
    zcode = json.loads((REPO_ROOT / ".zcode" / "config.json").read_text(encoding="utf-8"))
    for block in zcode.get("hooks", {}).get("events", {}).get("PreToolUse", []):
        for hook in block.get("hooks", []):
            commands.append(("zcode", hook["command"]))
    codex = tomllib.loads((REPO_ROOT / ".codex" / "config.toml").read_text(encoding="utf-8"))
    for block in codex.get("hooks", {}).get("PreToolUse", []):
        for hook in block.get("hooks", []):
            commands.append(("codex", hook["command"]))
    return commands


def _script_path(command: str) -> tuple[str, Path]:
    """从命令串提取解释器与仓内脚本路径（支持 $ 根占位与引号形态）。"""
    normalized = command.replace(ROOT_TOKEN, str(REPO_ROOT))
    match = re.search(r"(bash|python3?)\s+[\"']?([^\"'\s]+)[\"']?", normalized)
    if match is None:
        raise AssertionError(f"无法解析 hook 命令：{command}")
    return match.group(1), Path(match.group(2))


class RegisteredAdapterIntegrity(unittest.TestCase):
    """python3 调用的必须是真 Python；bash 调用的必须过 bash -n。"""

    def test_python_registered_scripts_parse(self) -> None:
        checked = 0
        for tool, command in _registered_commands():
            interpreter, path = _script_path(command)
            if not interpreter.startswith("python"):
                continue
            self.assertTrue(path.is_file(), f"{tool} 登记的脚本不存在：{command}")
            source = path.read_text(encoding="utf-8")
            try:
                ast.parse(source)
            except SyntaxError as exc:
                self.fail(f"{tool} 用 python3 调用但不是合法 Python（{path.name}）：{exc}")
            checked += 1
        self.assertGreaterEqual(checked, 1, "应至少登记一个 python 适配器")

    def test_bash_registered_scripts_pass_bash_n(self) -> None:
        for tool, command in _registered_commands():
            interpreter, path = _script_path(command)
            if interpreter != "bash":
                continue
            self.assertTrue(path.is_file(), f"{tool} 登记的脚本不存在：{command}")
            result = subprocess.run(["bash", "-n", str(path)], capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, f"{path} bash -n 失败：{result.stderr}")


class CodexConfigShape(unittest.TestCase):
    """形状锁：事件为数组表、命令嵌套、timeout 按秒、agent 用 config_file 注册。"""

    def setUp(self) -> None:
        self.config = tomllib.loads((REPO_ROOT / ".codex" / "config.toml").read_text(encoding="utf-8"))

    def test_pretooluse_is_array_of_tables(self) -> None:
        events = self.config["hooks"]["PreToolUse"]
        self.assertIsInstance(events, list)
        for block in events:
            self.assertIn("hooks", block)
            self.assertIsInstance(block["hooks"], list)

    def test_hook_command_fields(self) -> None:
        for block in self.config["hooks"]["PreToolUse"]:
            for hook in block["hooks"]:
                self.assertEqual(hook["type"], "command")
                self.assertIn("command", hook)
                self.assertIsInstance(hook["timeout"], int)
                self.assertNotIn("timeoutMs", hook, "codex timeout 按秒，不得混入毫秒字段")

    def test_agents_registered_via_config_file(self) -> None:
        for name, agent in self.config.get("agents", {}).items():
            if not isinstance(agent, dict) or "config_file" not in agent:
                continue
            self.assertTrue(
                (REPO_ROOT / ".codex" / agent["config_file"]).is_file(),
                f"agent {name} 的 config_file 不存在",
            )


class ZcodeConfigShape(unittest.TestCase):
    def setUp(self) -> None:
        self.config = json.loads((REPO_ROOT / ".zcode" / "config.json").read_text(encoding="utf-8"))

    def test_hooks_enabled_with_ms_timeout(self) -> None:
        hooks = self.config["hooks"]
        self.assertIs(hooks["enabled"], True, "缺失/关闭时 hook 静默不执行")
        for block in hooks["events"]["PreToolUse"]:
            for hook in block["hooks"]:
                self.assertIsInstance(hook["timeoutMs"], int, "ZCode 超时必须用毫秒字段 timeoutMs")
                self.assertNotIn("timeout", hook)
                _, path = _script_path(hook["command"])
                self.assertTrue(path.is_file(), f"zcode hook 脚本不存在：{hook['command']}")


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""hooks 安全门探针：用无副作用 JSON 输入验证允许/拒绝语义。"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
GATE = REPO_ROOT / ".claude" / "hooks" / "pre_tool_use_gate.py"
ADAPTER = REPO_ROOT / ".claude" / "hooks" / "block_dangerous.sh"


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


if __name__ == "__main__":
    unittest.main()

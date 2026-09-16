#!/usr/bin/env python3
"""resolver 行为验证：真实仓库解析 + 临时副本失败注入。"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
RESOLVER = REPO_ROOT / "scripts" / "resolve_agent_rules.py"


def run_resolver(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    return subprocess.run(
        [sys.executable, str(RESOLVER), *args],
        capture_output=True, text=True, env=env, cwd=str(cwd or REPO_ROOT), timeout=120,
    )


class RealRepoResolution(unittest.TestCase):
    def test_known_path_resolves_domain(self) -> None:
        result = run_resolver("term/src/lib.rs")
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = result.stdout.splitlines()
        self.assertIn("docs/AGENT_RULES/terminal-model.md", docs)

    def test_directory_scope_expands(self) -> None:
        result = run_resolver("mux")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("docs/AGENT_RULES/mux-domain.md", result.stdout.splitlines())

    def test_task_code_adds_comments_rule(self) -> None:
        result = run_resolver("term/src/lib.rs", "--task", "code")
        docs = result.stdout.splitlines()
        self.assertIn("docs/AGENT_RULES/terminal-model.md", docs)
        self.assertIn("docs/AGENT_RULES/code-comments.md", docs)

    def test_task_review_routes(self) -> None:
        result = run_resolver("term/src/lib.rs", "--task", "review")
        docs = result.stdout.splitlines()
        self.assertIn("docs/AGENT_RULES/code-review.md", docs)
        self.assertIn("docs/AGENT_RULES/terminal-model.md", docs)

    def test_json_payload_shape(self) -> None:
        result = run_resolver("--json", "vtparse/src/lib.rs")
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["version"], 1)
        ids = [rule["id"] for rule in payload["rules"]]
        self.assertIn("terminal-model", ids)
        self.assertEqual(ids, sorted(ids))

    def test_unknown_path_fails(self) -> None:
        result = run_resolver("zzz/does-not-exist.rs")
        self.assertEqual(result.returncode, 2)
        self.assertIn("error", result.stderr)

    def test_unknown_task_fails(self) -> None:
        result = run_resolver("term/src/lib.rs", "--task", "no-such-task")
        self.assertEqual(result.returncode, 2)


class TempRepoFailureInjection(unittest.TestCase):
    """临时副本验证守门真的会失败（不是只查标题齐全）。"""

    def _make_repo(self, root: Path, agents_bytes: int) -> None:
        root.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        (root / "AGENTS.md").write_text("# t\n" + "x" * max(agents_bytes - 5, 1), encoding="utf-8")
        rules_dir = root / "docs" / "AGENT_RULES"
        rules_dir.mkdir(parents=True)
        (rules_dir / "alpha.md").write_text("# alpha\n范围与不变量。" * 5, encoding="utf-8")
        (rules_dir / "routes.toml").write_text(
            'version = 2\nroot_max_bytes = 16384\n'
            'root_only = ["AGENTS.md", "README.md", "docs/AGENT_RULES/**"]\n\n'
            '[[rules]]\nid = "alpha"\ndoc = "docs/AGENT_RULES/alpha.md"\n'
            'paths = ["src/**"]\ntasks = []\n',
            encoding="utf-8",
        )
        src = root / "src"
        src.mkdir()
        (src / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
        (root / "README.md").write_text("r\n", encoding="utf-8")
        env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True, env=env)
        subprocess.run(
            ["git", "-C", str(root), "-c", "user.email=t@t", "-c", "user.name=t",
             "commit", "-qm", "init"], check=True, env=env,
        )

    def test_check_passes_on_valid_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            self._make_repo(root, agents_bytes=100)
            result = subprocess.run(
                [sys.executable, str(RESOLVER), "--check", "--root", str(root)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_check_rejects_oversized_agents(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            self._make_repo(root, agents_bytes=17000)
            result = subprocess.run(
                [sys.executable, str(RESOLVER), "--check", "--root", str(root)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("超过", result.stderr)

    def test_check_rejects_uncovered_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            self._make_repo(root, agents_bytes=100)
            (root / "orphan.txt").write_text("x\n", encoding="utf-8")
            env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
            subprocess.run(["git", "-C", str(root), "add", "orphan.txt"], check=True, env=env)
            result = subprocess.run(
                [sys.executable, str(RESOLVER), "--check", "--root", str(root)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("未声明路由", result.stderr)


if __name__ == "__main__":
    unittest.main()

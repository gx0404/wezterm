#!/usr/bin/env python3
import subprocess
import sys
from pathlib import Path


if __name__ == "__main__":
    gate = Path(__file__).resolve().parents[2] / ".claude/hooks/pre_tool_use_gate.py"
    raise SystemExit(subprocess.call([sys.executable, str(gate), "--protocol", "codex", *sys.argv[1:]]))

#!/usr/bin/env python3
"""AI 知识库构建器：受控语料 -> kb/chunks.json。

语料（闭集，见 CORPUS 列表）：根规则、fork CHANGELOG、贡献指南、全部领域
规则、七份框架手册，外加从 workspace Cargo.toml 生成的 crate 地图段。
输出契约：无时间戳、无绝对路径、键排序稳定，source_hashes 登记每个输入的
sha256——同样的输入必然字节一致。

用法：build_agent_kb.py            # 只读校验（与磁盘不一致时退出 1）
      build_agent_kb.py --confirm  # 写盘（有意重建时）
      build_agent_kb.py --check    # 同默认（显式别名）
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ModuleNotFoundError:
        raise SystemExit("需要 Python 3.11+ 或项目环境（make setup 提供 tomli）") from None

REPO_ROOT = Path(__file__).resolve().parents[1]
KB_PATH = REPO_ROOT / "kb" / "chunks.json"
SCHEMA_VERSION = 1
MAX_CHUNK_CHARS = 1500

MANUAL_DOCS = [
    "docs/README.md",
    "docs/ARCHITECTURE.md",
    "docs/DEVELOPMENT.md",
    "docs/MAKE_COMMANDS.md",
    "docs/TESTING.md",
    "docs/AI_TOOLS.md",
    "docs/RELEASE.md",
]


def _corpus_files() -> list[str]:
    files = ["AGENTS.md", "CHANGELOG.md", "CONTRIBUTING.md"]
    rules = sorted((REPO_ROOT / "docs" / "AGENT_RULES").glob("*.md"))
    files.extend(path.relative_to(REPO_ROOT).as_posix() for path in rules)
    files.extend(MANUAL_DOCS)
    missing = [name for name in files if not (REPO_ROOT / name).is_file()]
    if missing:
        raise SystemExit(f"语料缺失：{missing}")
    return files


def _crate_map_markdown() -> str:
    """从 workspace Cargo.toml 生成 crate 地图段（生成内容，非手写）。"""
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    members = sorted(manifest.get("workspace", {}).get("members", []))
    lines = ["# Crate 地图（生成）", "", "由 scripts/build_agent_kb.py 从根 Cargo.toml 生成；每行是目录与包描述。", ""]
    for member in members:
        cargo = REPO_ROOT / member / "Cargo.toml"
        if not cargo.is_file():
            continue
        try:
            package = tomllib.loads(cargo.read_text(encoding="utf-8")).get("package", {})
        except Exception:  # noqa: BLE001 - 单个畸形清单不阻塞整体
            package = {}
        name = package.get("name", member)
        desc = (package.get("description") or "").strip().splitlines()
        desc_text = desc[0].strip() if desc else ""
        lines.append(f"- `{member}/` ({name})：{desc_text}" if desc_text else f"- `{member}/` ({name})")
    lines.append("")
    return "\n".join(lines)


def _split_by_headings(text: str) -> list[tuple[str, str]]:
    """按 ATX 标题切块，返回 (锚点, 正文)。无标题整体一块。"""
    sections: list[tuple[str, list[str]]] = [("顶", [])]
    for line in text.splitlines():
        if line.startswith("#"):
            sections.append((line.lstrip("#").strip() or "无题", [line]))
        else:
            sections[-1][1].append(line)
    result = []
    for heading, body in sections:
        result.append((heading, "\n".join(body).strip()))
    return [(heading, body) for heading, body in result if body]


def _split_long(anchor: str, body: str) -> list[str]:
    if len(body) <= MAX_CHUNK_CHARS:
        return [body]
    parts: list[str] = []
    current: list[str] = []
    length = 0
    for paragraph in body.split("\n\n"):
        chunk = paragraph if not current else "\n\n" + paragraph
        if length + len(chunk) > MAX_CHUNK_CHARS and current:
            parts.append("\n\n".join(current))
            current, length = [], 0
            chunk = paragraph
        current.append(paragraph)
        length += len(chunk)
    if current:
        parts.append("\n\n".join(current))
    return parts


def build_chunks() -> dict:
    sources = _corpus_files()
    chunks: list[dict] = []
    hashes: dict[str, str] = {}
    for name in sources:
        raw = (REPO_ROOT / name).read_text(encoding="utf-8")
        hashes[name] = hashlib.sha256(raw.encode("utf-8")).hexdigest()
        for heading, body in _split_by_headings(raw):
            for index, part in enumerate(_split_long(heading, body)):
                chunk_id = f"{name}#{heading}" if index == 0 else f"{name}#{heading}[{index}]"
                chunks.append({"id": chunk_id, "source": name, "anchor": heading, "text": part})
    crate_map = _crate_map_markdown()
    hashes["<generated>crate-map"] = hashlib.sha256(crate_map.encode("utf-8")).hexdigest()
    for heading, body in _split_by_headings(crate_map):
        for index, part in enumerate(_split_long(heading, body)):
            chunk_id = f"crate-map#{heading}" if index == 0 else f"crate-map#{heading}[{index}]"
            chunks.append({"id": chunk_id, "source": "crate-map", "anchor": heading, "text": part})
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_by": "scripts/build_agent_kb.py",
        "source_hashes": dict(sorted(hashes.items())),
        "chunks": sorted(chunks, key=lambda item: item["id"]),
    }


def render(payload: dict) -> bytes:
    return (json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--confirm", action="store_true", help="写盘（默认只读校验）")
    parser.add_argument("--check", action="store_true", help="显式只读校验（与默认相同）")
    args = parser.parse_args(argv)

    payload = build_chunks()
    canonical = render(payload)
    target = KB_PATH
    if args.confirm:
        target.parent.mkdir(parents=True, exist_ok=True)
        tmp = target.with_suffix(".json.tmp")
        tmp.write_bytes(canonical)
        tmp.replace(target)
        print(f"kb written: {target.relative_to(REPO_ROOT)}（{len(payload['chunks'])} chunks，{len(payload['source_hashes'])} 源）")
        return 0
    if not target.is_file():
        print(f"error: 缺少 {target.relative_to(REPO_ROOT)}；运行 make kb 生成", file=sys.stderr)
        return 1
    on_disk = target.read_bytes()
    if on_disk == canonical:
        print(f"kb OK: {len(payload['chunks'])} chunks，与语料一致")
        return 0
    try:
        old = json.loads(on_disk)
        old_hashes = old.get("source_hashes", {})
        new_hashes = payload["source_hashes"]
        changed = sorted(set(old_hashes) ^ set(new_hashes)) + sorted(
            name for name in old_hashes.keys() & new_hashes.keys() if old_hashes[name] != new_hashes[name]
        )
        print(f"error: kb 与语料不一致；差异源：{changed or '结构性变更（chunk 数不同）'}；运行 make kb 重建", file=sys.stderr)
    except Exception:  # noqa: BLE001 - 磁盘内容非 JSON 时给出直白错误
        print("error: kb/chunks.json 不是有效 JSON；运行 make kb 重建", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

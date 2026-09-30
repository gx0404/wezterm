#!/usr/bin/env python3
"""Generate the launcher table from the checked-in release registry.

Normal generation and --check work in shallow clones and source archives without
Git or a parent CHANGELOG. --record appends an immutable release from an explicit
Git checkout; --verify-git audits the registry against available release objects.
GX Shell can opt into its release-coverage gate with --gx-shell-changelog PATH.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

from gx_package import CONFIG_USER_DATA, SNAPSHOT_SKIPPED_PARTS

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "scripts/gx-launcher/released.rs"
REGISTRY = ROOT / "scripts/gx-config-releases.json"
ALGORITHM = "fnv1a64-crlf-v1"


def relative_path(value: str) -> bool:
    return (isinstance(value, str) and bool(value) and "\\" not in value
            and all(part not in {"", ".", ".."} for part in value.split("/"))
            and not any(ord(char) < 32 or char in '":' for char in value))


def validate_registry(registry: dict) -> dict:
    if (not isinstance(registry, dict) or type(registry.get("schema")) is not int
            or registry["schema"] != 1 or registry.get("algorithm") != ALGORITHM):
        raise ValueError("unsupported release registry schema or fingerprint algorithm")
    releases = registry.get("releases")
    if not isinstance(releases, list) or not 1 <= len(releases) <= 8:
        raise ValueError("release registry must contain 1..8 releases (launcher mask is u8)")
    names = set()
    for release in releases:
        if not isinstance(release, dict):
            raise ValueError("release must be an object")
        name, repository, tag = (release.get(key) for key in ("name", "repository", "tag"))
        if not all(isinstance(value, str) and value and not any(ord(c) < 32 for c in value)
                   for value in (name, repository, tag)):
            raise ValueError("release name, repository and tag are required single-line strings")
        if not repository.startswith("https://") or not relative_path(tag):
            raise ValueError(f"invalid release provenance: {name}")
        identity = name.split(" (")[0]
        if identity in names:
            raise ValueError(f"duplicate release: {name}")
        names.add(identity)
        for key in ("commit", "tree"):
            if not re.fullmatch(r"[0-9a-f]{40}", str(release.get(key, ""))):
                raise ValueError(f"{name}: {key} must be a full Git object id")
        if not relative_path(release.get("prefix")):
            raise ValueError(f"{name}: invalid config tree prefix")
        files = release.get("files")
        if not isinstance(files, list) or not files:
            raise ValueError(f"{name}: no released files")
        paths = []
        for entry in files:
            if not isinstance(entry, dict):
                raise ValueError(f"{name}: file entry must be an object")
            path = entry.get("path")
            if (not relative_path(path) or path in CONFIG_USER_DATA
                    or SNAPSHOT_SKIPPED_PARTS.intersection(path.split("/"))):
                raise ValueError(f"{name}: invalid or user-owned config path {path!r}")
            if type(entry.get("length")) is not int or entry["length"] < 0:
                raise ValueError(f"{name}: invalid length for {path}")
            if not re.fullmatch(r"[0-9a-f]{16}", str(entry.get("fnv1a64", ""))):
                raise ValueError(f"{name}: invalid fingerprint for {path}")
            if not re.fullmatch(r"[0-9a-f]{40}", str(entry.get("blob", ""))):
                raise ValueError(f"{name}: invalid blob for {path}")
            paths.append(path)
        if paths != sorted(set(paths)):
            raise ValueError(f"{name}: released paths must be sorted and unique")
    return registry


def load_registry(path: Path = REGISTRY) -> dict:
    return validate_registry(json.loads(path.read_text(encoding="utf-8")))


def git(*args: str, root: Path = ROOT, data: bytes | None = None) -> bytes:
    return subprocess.run(["git", "-C", str(root), *args], input=data, capture_output=True, check=True).stdout


def published_versions(changelog: str) -> list[str]:
    """Dated GX Shell headings older than its newest release-in-preparation heading."""
    headings = [(tuple(map(int, version.split("."))), version, date)
                for version, date in re.findall(r"^## (\d+\.\d+\.\d+)\(([^)]*)\)", changelog, re.MULTILINE)]
    newest = max((key for key, _, _ in headings), default=None)
    return [version for key, version, date in headings
            if key != newest and re.fullmatch(r"\d{4}-\d{2}-\d{2}", date)]


def unlisted_releases(changelog: str, releases=None) -> list[str]:
    if releases is None:
        releases = load_registry()["releases"]
    listed = {release["name"].split(" (")[0] for release in releases}
    return [f"GX Shell {version}" for version in published_versions(changelog)
            if f"GX Shell {version}" not in listed]


def fnv1a(data: bytes) -> int:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def fingerprint(data: bytes) -> tuple[int, int]:
    """Same normalization as main.rs::fingerprint: NUL marks binary, otherwise CRLF becomes LF."""
    if b"\0" not in data:
        data = data.replace(b"\r\n", b"\n")
    return len(data), fnv1a(data)


def release_files(commit: str, prefix: str, root: Path = ROOT) -> dict[str, str]:
    files = {}
    for record in git("ls-tree", "-r", "-z", "--full-tree", f"{commit}:{prefix}", root=root).split(b"\0"):
        if not record:
            continue
        meta, path = record.decode("utf-8").split("\t", 1)
        mode, kind, blob = meta.split()
        if path in CONFIG_USER_DATA or SNAPSHOT_SKIPPED_PARTS.intersection(path.split("/")):
            continue
        if kind != "blob" or mode not in {"100644", "100755"}:
            raise ValueError(f"{commit}:{prefix}/{path} is not a regular file (mode {mode})")
        files[path] = blob
    return files


def blob_contents(blobs: set[str], root: Path = ROOT) -> dict[str, bytes]:
    order = sorted(blobs)
    out = git("cat-file", "--batch", root=root, data="".join(f"{blob}\n" for blob in order).encode("ascii"))
    contents, pos = {}, 0
    for blob in order:
        end = out.index(b"\n", pos)
        name, kind, size = out[pos:end].decode("ascii").split()
        if name != blob or kind != "blob":
            raise ValueError(f"unexpected git object {name} ({kind})")
        start = end + 1
        contents[blob] = out[start:start + int(size)]
        pos = start + int(size) + 1
    return contents


def collect_release(name: str, repository: str, tag: str, commit: str, prefix: str, root: Path) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not relative_path(prefix) or not relative_path(tag):
        raise ValueError("recording requires a full commit id and relative prefix/tag")
    git("cat-file", "-e", f"{commit}^{{commit}}", root=root)
    tree = git("rev-parse", f"{commit}:{prefix}", root=root).decode("ascii").strip()
    files = release_files(commit, prefix, root)
    data = blob_contents(set(files.values()), root)
    hashes = {blob: fingerprint(content) for blob, content in data.items()}
    release = dict(name=name, repository=repository, tag=tag, commit=commit, prefix=prefix, tree=tree,
                   files=[dict(path=path, blob=blob, length=hashes[blob][0], fnv1a64=f"{hashes[blob][1]:016x}")
                          for path, blob in sorted(files.items())])
    validate_registry(dict(schema=1, algorithm=ALGORITHM, releases=[release]))
    return release


def verify_git(releases: list[dict], root: Path) -> None:
    for release in releases:
        actual = collect_release(*(release[key] for key in ("name", "repository", "tag", "commit", "prefix")), root)
        if actual != release:
            raise ValueError(f"release registry differs from Git objects: {release['name']}")
        tag = subprocess.run(["git", "-C", str(root), "rev-parse", "-q", "--verify",
                              f"refs/tags/{release['tag']}^{{commit}}"], capture_output=True, text=True)
        if tag.returncode == 0 and tag.stdout.strip() != release["commit"]:
            raise ValueError(f"release tag points to another commit: {release['tag']}")


def released_entries(releases=None) -> list[tuple[str, int, int, int]]:
    if releases is None:
        releases = load_registry()["releases"]
    masks: dict[tuple[str, int, int], int] = {}
    for bit, release in enumerate(releases):
        for entry in release["files"]:
            key = (entry["path"], entry["length"], int(entry["fnv1a64"], 16))
            masks[key] = masks.get(key, 0) | 1 << bit
    return sorted((*key, mask) for key, mask in masks.items())


def render(entries: list[tuple[str, int, int, int]], releases=None) -> str:
    if releases is None:
        releases = load_registry()["releases"]
    lines = [
        "// @generated by scripts/gx_config_fingerprints.py from gx-config-releases.json; do not edit.",
        "// (path, length, FNV-1a 64, releases) of every wezterm-config file shipped by a release,",
        "// hashed with the same text/binary normalization as main.rs::fingerprint. Bit i of",
        "// releases marks the i-th release below as shipping exactly that content:",
        *(line for bit, release in enumerate(releases)
          for line in (f"// {bit}. {release['name']}", f"//    {release['commit']}:{release['prefix']}")),
        "#[rustfmt::skip]",
        "pub const RELEASED: &[(&str, usize, u64, u8)] = &[",
    ]
    for path, length, value, mask in entries:
        digits = f"{value:016x}"
        grouped = "_".join(digits[i:i + 4] for i in range(0, 16, 4))
        lines.append(f'    ("{path}", {length}, 0x{grouped}, 0b{mask:0{len(releases)}b}),')
    lines.append("];")
    return "\n".join(lines) + "\n"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="compare released.rs to the local registry without Git")
    mode.add_argument("--record", action="store_true", help="append a release from immutable Git objects")
    parser.add_argument("--verify-git", action="store_true", help="require and audit every registered release object")
    parser.add_argument("--source-repo", type=Path, default=ROOT)
    parser.add_argument("--gx-shell-changelog", type=Path, help="explicit GX Shell release-coverage gate")
    for key in ("name", "repository", "tag", "commit", "prefix"):
        parser.add_argument(f"--{key}")
    args = parser.parse_args(argv)
    metadata = [getattr(args, key) for key in ("name", "repository", "tag", "commit", "prefix")]
    if args.record and not all(metadata):
        parser.error("--record requires --name, --repository, --tag, --commit and --prefix")
    if not args.record and any(metadata):
        parser.error("release metadata requires --record")
    try:
        registry = load_registry()
        if args.record:
            release = collect_release(*metadata, args.source_repo)
            tagged = git("rev-parse", "--verify", f"refs/tags/{args.tag}^{{commit}}", root=args.source_repo).decode().strip()
            if tagged != args.commit:
                raise ValueError(f"release tag does not match commit: {args.tag}")
            registry["releases"].append(release)
            validate_registry(registry)
        releases = registry["releases"]
        if args.gx_shell_changelog:
            unlisted = unlisted_releases(args.gx_shell_changelog.read_text(encoding="utf-8"), releases)
            if unlisted:
                raise ValueError(f"GX Shell releases missing from registry: {', '.join(unlisted)}")
        if args.verify_git:
            verify_git(releases, args.source_repo)
        entries = released_entries(releases)
        text = render(entries, releases)
        if args.check:
            current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.is_file() else ""
            if current != text:
                print(f"{OUTPUT} is stale; run {Path(__file__).name}", file=sys.stderr)
                return 1
            return 0
        if args.record:
            REGISTRY.write_text(json.dumps(registry, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        OUTPUT.write_text(text, encoding="utf-8", newline="\n")
        print(f"wrote {OUTPUT} ({len(entries)} entries)")
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())

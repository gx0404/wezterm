#!/usr/bin/env python3
"""Generate the launcher's table of released wezterm-config files.

The GX launcher replaces a user's config file only when it is identical to a file shipped by a
release (text compared with CRLF read as LF, binaries exactly). This script writes that table,
scripts/gx-launcher/released.rs, from the git objects of the release commits; it never reads the
working tree. After publishing a release, append it to RELEASES and rerun the script; both modes
refuse to run while the monorepo CHANGELOG.md dates a GX Shell release that RELEASES lacks.

    python3 scripts/gx_config_fingerprints.py           # rewrite released.rs
    python3 scripts/gx_config_fingerprints.py --check   # fail if released.rs is stale
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

from gx_package import CONFIG_USER_DATA, SNAPSHOT_SKIPPED_PARTS

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "scripts/gx-launcher/released.rs"
CHANGELOG = ROOT.parent / "CHANGELOG.md"
# (release, commit, wezterm-config tree in that commit). Component tags are not fetched into the
# gx_shell monorepo, so releases are pinned by commit.
RELEASES = (
    ("WezTerm GX 0.3.0 (gx0404/wezterm tag gx-v0.3.0)", "97d2768b288a16a854f733bfb45ed8f8bf18173a",
     "dotfiles/wezterm-config"),
    ("GX Shell 0.1.0 (tag gx-shell-v0.1.0)", "b0f5d696e5a8b9ce8efa03cb0269525056499c08",
     "wezterm/dotfiles/wezterm-config"),
    ("GX Shell 0.2.0 (tag gx-shell-v0.2.0)", "ee1905f121c6671f86aa7f3b3761ebb18053cf1a",
     "wezterm/dotfiles/wezterm-config"),
)
_FINGERPRINTS: dict[str, tuple[int, int]] = {}


def git(*args: str, data: bytes | None = None) -> bytes:
    return subprocess.run(["git", "-C", str(ROOT), *args], input=data, capture_output=True, check=True).stdout


def available(commit: str) -> bool:
    return subprocess.run(["git", "-C", str(ROOT), "cat-file", "-e", f"{commit}^{{commit}}"],
                          capture_output=True).returncode == 0


def published_versions(changelog: str) -> list[str]:
    """Versions of the dated `## X.Y.Z(YYYY-MM-DD)` headings older than the newest heading, which
    is the release in preparation: each was published and must be in RELEASES."""
    headings = [(tuple(map(int, version.split("."))), version, date)
                for version, date in re.findall(r"^## (\d+\.\d+\.\d+)\(([^)]*)\)", changelog, re.MULTILINE)]
    newest = max((key for key, _, _ in headings), default=None)
    return [version for key, version, date in headings
            if key != newest and re.fullmatch(r"\d{4}-\d{2}-\d{2}", date)]


def unlisted_releases(changelog: str, releases=RELEASES) -> list[str]:
    listed = {name.split(" (")[0] for name, _, _ in releases}
    return [f"GX Shell {version}" for version in published_versions(changelog)
            if f"GX Shell {version}" not in listed]


def fnv1a(data: bytes) -> int:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def fingerprint(data: bytes) -> tuple[int, int]:
    """Same rule as main.rs::fingerprint: NUL bytes mark binaries, text is hashed with CRLF as LF."""
    if b"\0" not in data:
        data = data.replace(b"\r\n", b"\n")
    return len(data), fnv1a(data)


def release_files(commit: str, prefix: str) -> dict[str, str]:
    """Map each shipped path (relative to wezterm-config) to its blob id."""
    files = {}
    for record in git("ls-tree", "-r", "-z", "--full-tree", f"{commit}:{prefix}").split(b"\0"):
        if not record:
            continue
        meta, path = record.decode("utf-8").split("\t", 1)
        mode, kind, blob = meta.split()
        if kind != "blob" or path in CONFIG_USER_DATA or SNAPSHOT_SKIPPED_PARTS.intersection(path.split("/")):
            continue
        if mode not in {"100644", "100755"}:
            raise ValueError(f"{commit}:{prefix}/{path} is not a regular file (mode {mode})")
        files[path] = blob
    return files


def blob_contents(blobs: set[str]) -> dict[str, bytes]:
    order = sorted(blobs)
    out = git("cat-file", "--batch", data="".join(f"{blob}\n" for blob in order).encode("ascii"))
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


def released_entries(releases=RELEASES) -> list[tuple[str, int, int, int]]:
    """(path, length, FNV-1a 64, bitmask of the releases that shipped exactly this content)."""
    trees = [release_files(commit, prefix) for _, commit, prefix in releases]
    blobs = {blob for tree in trees for blob in tree.values()} - _FINGERPRINTS.keys()
    _FINGERPRINTS.update((blob, fingerprint(data)) for blob, data in blob_contents(blobs).items())
    masks: dict[tuple[str, int, int], int] = {}
    for bit, tree in enumerate(trees):
        for path, blob in tree.items():
            key = (path, *_FINGERPRINTS[blob])
            masks[key] = masks.get(key, 0) | 1 << bit
    return sorted((*key, mask) for key, mask in masks.items())


def render(entries: list[tuple[str, int, int, int]], releases=RELEASES) -> str:
    lines = [
        "// @generated by scripts/gx_config_fingerprints.py from git; do not edit.",
        "// (path, length, FNV-1a 64, releases) of every wezterm-config file shipped by a release,",
        "// hashed with the same text/binary normalization as main.rs::fingerprint. Bit i of",
        "// releases marks the i-th release below as shipping exactly that content:",
        *(line for bit, (name, commit, prefix) in enumerate(releases)
          for line in (f"// {bit}. {name}", f"//    {commit}:{prefix}")),
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
    parser.add_argument("--check", action="store_true", help="fail if released.rs differs from git")
    args = parser.parse_args(argv)
    unlisted = unlisted_releases(CHANGELOG.read_text(encoding="utf-8") if CHANGELOG.is_file() else "")
    if unlisted:
        print(f"{CHANGELOG.name} dates {', '.join(unlisted)}; add them to RELEASES with their tag commits",
              file=sys.stderr)
        return 1
    missing = [commit for _, commit, _ in RELEASES if not available(commit)]
    if missing:
        print(f"release commits are not in this clone; fetch them: {', '.join(missing)}", file=sys.stderr)
        return 2
    entries = released_entries()
    text = render(entries)
    if args.check:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.is_file() else ""
        if current != text:
            print(f"{OUTPUT} is stale; run {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    OUTPUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUTPUT} ({len(entries)} entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

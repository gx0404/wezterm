#!/usr/bin/env python3
"""Validate GX build artifacts and publish only from the manual fork workflow."""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from gx_package import BINARIES, MAX_GLIBC, ROOT, digest, output, source_info, validate_version

REPOSITORY = "gx0404/wezterm"


def tag_commit(tag: str) -> str | None:
    import subprocess
    result = subprocess.run(["git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"],
                            cwd=ROOT, capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def verify_tag(existing: str | None, expected: str):
    if existing is not None and existing != expected:
        raise ValueError("release tag already points to a different commit; it will not be moved")


def verify_artifacts(folder: Path, version: str, sha: str) -> list[Path]:
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("expected a complete source commit SHA")
    names = [(f"WezTerm-GX-{version}-Setup-x64.exe", "windows"),
             (f"wezterm-gx_{version}_amd64.deb", "deb")]
    files, resource_versions, product_versions = [], set(), set()
    for name, kind in names:
        artifact = folder / name
        metadata = folder / (name + ".manifest.json")
        checksums = folder / (name + ".sha256")
        for path in (artifact, metadata, checksums):
            if not path.is_file() or path.is_symlink():
                raise ValueError(f"missing release artifact: {path.name}")
        info = json.loads(metadata.read_text(encoding="utf-8"))
        expected = {"schema": 1, "package_version": version, "source_commit": sha,
                    "source_dirty": False, "platform": kind, "architecture": "amd64", "artifact": name}
        for key, value in expected.items():
            if info.get(key) != value:
                raise ValueError(f"{name}: manifest {key} differs from the release inputs")
        if kind == "deb":
            compatibility = info.get("linux_compatibility", {})
            if (compatibility.get("build_ubuntu") != "20.04"
                    or compatibility.get("supported_ubuntu") != ["20.04", "24.04"]
                    or compatibility.get("openssl") != "static"):
                raise ValueError(f"{name}: missing portable Ubuntu compatibility audit")
            elf = compatibility.get("elf", {})
            if set(elf) != set(BINARIES) | {"wezterm-gx", "wezterm-gx-gui"}:
                raise ValueError(f"{name}: incomplete Linux executable audit")
            for binary, audit in elf.items():
                glibc = audit.get("max_glibc", "")
                if (not re.fullmatch(r"\d+(?:\.\d+)+", glibc)
                        or tuple(map(int, glibc.split("."))) > MAX_GLIBC
                        or not isinstance(audit.get("needed"), list)
                        or any(not isinstance(lib, str) or re.match(r"lib(?:ssl|crypto)\.so", lib)
                               for lib in audit["needed"])):
                    raise ValueError(f"{name}: {binary} is not portable to Ubuntu 20.04/24.04")
        if info.get("sha256") != digest(artifact):
            raise ValueError(f"{name}: artifact SHA-256 mismatch")
        expected_sums = f"{digest(artifact)}  {name}\n{digest(metadata)}  {metadata.name}\n"
        if checksums.read_text(encoding="ascii") != expected_sums:
            raise ValueError(f"{name}: checksum file mismatch")
        if not re.fullmatch(r"[0-9a-f]{64}", info.get("resource_version", "")):
            raise ValueError(f"{name}: invalid resource version")
        product = info.get("product_version", "")
        if not re.fullmatch(r"\d{8}-\d{6}-[0-9a-f]{8,40}", product) or not sha.startswith(product.rsplit("-", 1)[1]):
            raise ValueError(f"{name}: product is not built from the requested commit")
        resource_versions.add(info["resource_version"])
        product_versions.add(product)
        files.extend([artifact, metadata, checksums])
    if len(resource_versions) != 1 or len(product_versions) != 1:
        raise ValueError("Windows and Linux artifacts do not contain the same source/resources")
    return files


class GitHub:
    def __init__(self):
        self.token = os.environ["GH_TOKEN"]
        self.base = f"https://api.github.com/repos/{REPOSITORY}"

    def request(self, method: str, path: str, data=None, binary: Path | None = None):
        url = path if path.startswith("https://uploads.github.com/") else self.base + path
        body = binary.read_bytes() if binary else (json.dumps(data).encode() if data is not None else None)
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json",
                   "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "wezterm-gx-release",
                   "Content-Type": "application/octet-stream" if binary else "application/json"}
        request = urllib.request.Request(url, data=body, headers=headers, method=method)
        with urllib.request.urlopen(request, timeout=180) as response:
            payload = response.read()
            return json.loads(payload) if payload else None

    def optional(self, path: str):
        try:
            return self.request("GET", path)
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None
            raise


def remote_commit(api: GitHub, tag: str) -> str | None:
    ref = api.optional("/git/ref/tags/" + urllib.parse.quote(tag, safe=""))
    if ref is None:
        return None
    obj = ref["object"]
    for _ in range(8):
        if obj["type"] == "commit":
            return obj["sha"]
        if obj["type"] != "tag":
            break
        obj = api.request("GET", "/git/tags/" + obj["sha"])["object"]
    raise ValueError("release tag does not resolve to a commit")


def publish(folder: Path, version: str, sha: str):
    if (os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
            or os.environ.get("GITHUB_REPOSITORY") != REPOSITORY):
        raise ValueError("publishing is only enabled in the manual gx0404/wezterm Actions workflow")
    files = verify_artifacts(folder, version, sha)
    tag = f"gx-v{version}"
    api = GitHub()
    existing_commit = remote_commit(api, tag)
    verify_tag(existing_commit, sha)
    release = api.optional("/releases/tags/" + urllib.parse.quote(tag, safe=""))
    if release and not release["draft"]:
        raise ValueError("this version is already published; choose a new version")
    if existing_commit is None:
        api.request("POST", "/git/refs", {"ref": "refs/tags/" + tag, "sha": sha})
    if release is None:
        release = api.request("POST", "/releases", {
            "tag_name": tag, "target_commitish": sha, "name": f"WezTerm GX {version}", "draft": True,
            "body": f"Source: `{sha}`\n\nWindows x64 setup (current user or all users) and one amd64 deb for Ubuntu 20.04 and 24.04. "
                    "Includes GX configuration, plugins, fonts and wallpapers. Existing user configuration and sessions are preserved.\n\n"
                    "Windows packages are unsigned. Verify downloads using the attached SHA-256 files.",
        })
    # Resume an interrupted draft only when its immutable tag still names this SHA.
    assets = {a["name"]: a for a in api.request("GET", f'/releases/{release["id"]}/assets?per_page=100')}
    upload = release["upload_url"].split("{", 1)[0]
    for path in files:
        if path.name in assets:
            asset = assets[path.name]
            if asset.get("digest") == "sha256:" + digest(path):
                continue
            raise ValueError(f"draft already has a different asset: {path.name}; use a new release version")
        api.request("POST", upload + "?name=" + urllib.parse.quote(path.name, safe=""), binary=path)
    # Check the remote list before making a partially uploaded draft public.
    uploaded = api.request("GET", f'/releases/{release["id"]}/assets?per_page=100')
    if {a["name"] for a in uploaded if a["state"] == "uploaded"} != {p.name for p in files}:
        raise ValueError("draft assets are incomplete or unexpected; draft remains unpublished")
    if remote_commit(api, tag) != sha:
        raise ValueError("release tag changed or disappeared; draft remains unpublished")
    result = api.request("PATCH", f'/releases/{release["id"]}', {"draft": False})
    print(result["html_url"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "verify", "publish"])
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha")
    parser.add_argument("--artifacts", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    try:
        version = validate_version(args.version)
        if args.action == "prepare":
            sha, dirty = source_info()
            if dirty:
                raise ValueError("release preparation requires a clean checkout")
            verify_tag(tag_commit(f"gx-v{version}"), sha)
            record = f"sha={sha}\nversion={version}\ntag=gx-v{version}\n"
            print(record, end="")
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
                    stream.write(record)
        elif args.action == "verify":
            files = verify_artifacts(args.artifacts, version, args.sha or "")
            print(f"PASS: verified {len(files)} release files")
        else:
            publish(args.artifacts, version, args.sha or "")
        return 0
    except (ValueError, OSError, KeyError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

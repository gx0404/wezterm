#!/usr/bin/env python3
"""Build GX installers without invoking the upstream publishing scripts."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINARIES = ("wezterm", "wezterm-gui", "wezterm-mux-server", "strip-ansi-escapes")
WINDOWS_RUNTIME = {
    "conhost/conpty.dll": "conpty.dll",
    "conhost/OpenConsole.exe": "OpenConsole.exe",
    "angle/libEGL.dll": "libEGL.dll",
    "angle/libGLESv2.dll": "libGLESv2.dll",
    "mesa/opengl32.dll": "mesa/opengl32.dll",
}
RUNTIME_DEPS = (
    "ca-certificates", "fontconfig", "zsh", "desktop-file-utils", "libegl1", "libgl1",
    "libwayland-client0", "libwayland-cursor0", "libwayland-egl1",
    "libxkbcommon0", "libxkbcommon-x11-0", "libxcb-image0",
    "libxcb-keysyms1", "libxcb-render-util0", "libxcb-icccm4", "libxcb-ewmh2",
)
LINUX_BASELINE = "20.04"
MAX_GLIBC = (2, 31)
RUST_VERSION = "1.96.1"


def audit_linux_binary(path: Path) -> dict:
    symbols = output([tool("objdump"), "-T", path])
    versions = {tuple(map(int, value.split(".")))
                for value in re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", symbols)}
    if not versions or max(versions) > MAX_GLIBC or "GLIBC_PRIVATE" in symbols:
        raise ValueError(f"{path.name}: requires a newer/unknown glibc; rebuild on Ubuntu {LINUX_BASELINE}")
    dynamic = output([tool("objdump"), "-p", path])
    needed = sorted(re.findall(r"^\s*NEEDED\s+(\S+)", dynamic, re.M))
    if any(re.match(r"lib(?:ssl|crypto)\.so", name) for name in needed):
        raise ValueError(f"{path.name}: dynamic OpenSSL cannot span Ubuntu 20.04/24.04; rebuild with vendored-openssl")
    return {"max_glibc": ".".join(map(str, max(versions))), "needed": needed}


def run(argv, **kwargs):
    return subprocess.run([str(arg) for arg in argv], check=True, **kwargs)


def output(argv, **kwargs) -> str:
    return run(argv, capture_output=True, text=True, encoding="utf-8", **kwargs).stdout.strip()


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def version_from_changelog(root: Path = ROOT) -> str:
    matches = re.findall(r"^## (\d+)\.(\d+)\.(\d+)\((?:TBD|[\d-]+)\)\s*$",
                         (root / "CHANGELOG.md").read_text(encoding="utf-8"), re.M)
    if not matches:
        raise ValueError("CHANGELOG.md has no GX version")
    return ".".join(map(str, max(tuple(map(int, v)) for v in matches)))


def validate_version(version: str | None, root: Path = ROOT) -> str:
    expected = version_from_changelog(root)
    version = (version or "").strip()
    if not version:
        return expected
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError(f"version must be X.Y.Z without a prefix; leave it empty to use CHANGELOG.md ({expected})")
    if version != expected:
        raise ValueError(f"requested version {version}, but this commit's CHANGELOG.md requires {expected}; "
                         f"leave version empty or enter {expected}. To release a different version, update CHANGELOG.md first")
    return version


def product_version_from_source(sha: str, root: Path = ROOT) -> str:
    timestamp = int(output(["git", "-C", root, "show", "-s", "--format=%ct", sha]))
    return datetime.fromtimestamp(timestamp, timezone.utc).strftime("%Y%m%d-%H%M%S-") + sha[:8]


def tool(name: str) -> Path:
    found = shutil.which(name)
    if found:
        return Path(found)
    if name in {"cargo", "rustc", "rustfmt"}:
        candidate = Path.home() / ".cargo/bin" / (name + (".exe" if os.name == "nt" else ""))
        if candidate.is_file():
            return candidate
    if name == "iscc":
        candidates = [os.environ.get("ISCC", ""), ROOT / ".local/tools/inno/ISCC.exe"]
        for env in ("ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"):
            base = Path(os.environ.get(env, "C:/Program Files (x86)"))
            candidates.extend([base / "Inno Setup 6/ISCC.exe", base / "Programs/Inno Setup 6/ISCC.exe"])
        for candidate in candidates:
            if candidate and Path(candidate).is_file():
                return Path(candidate)
    raise ValueError(f"required tool not found: {name}")


def source_info(root: Path = ROOT) -> tuple[str, bool]:
    sha = output(["git", "-C", root, "rev-parse", "HEAD"])
    dirty = bool(output(["git", "-C", root, "status", "--porcelain", "--untracked-files=normal"]))
    return sha, dirty


def snapshot_files(root: Path = ROOT):
    base = root / "dotfiles"
    for part in ("wezterm-config", "plugins", "fonts"):
        # Path ordering differs between Windows (case-insensitive) and POSIX.
        for path in sorted((base / part).rglob("*"), key=lambda p: p.relative_to(base).as_posix()):
            rel = path.relative_to(base)
            if any(p in {".git", "state", "__pycache__"} for p in rel.parts):
                continue
            if path.is_symlink():
                raise ValueError(f"snapshot must not contain symlinks: {rel}")
            if path.is_file():
                yield rel, path


def resource_version(root: Path = ROOT) -> str:
    h = hashlib.sha256()
    for rel, path in snapshot_files(root):
        h.update(rel.as_posix().encode("utf-8") + b"\0")
        h.update(bytes.fromhex(digest(path)))
    return h.hexdigest()


def copy_file(source: Path, destination: Path, executable: bool = False):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def stage_resources(dest: Path, root: Path = ROOT):
    for rel, path in snapshot_files(root):
        if rel.parts[0] != "fonts":
            copy_file(path, dest / "dotfiles" / rel)
    for name in ("PROVENANCE.md", "README.md"):
        copy_file(root / "dotfiles" / name, dest / "dotfiles" / name)
    (dest / "resource-version").write_text(resource_version(root) + "\n", encoding="ascii")


def verify_binaries(bin_dir: Path, kind: str, sha: str) -> tuple[str, dict]:
    versions, hashes = set(), {}
    suffix = ".exe" if kind == "windows" else ""
    for name in BINARIES:
        path = bin_dir / (name + suffix)
        if not path.is_file():
            raise ValueError(f"missing binary: {path}")
        with path.open("rb") as file:
            header = file.read(64)
            if kind == "windows":
                if header[:2] != b"MZ":
                    raise ValueError(f"not a Windows executable: {path}")
                file.seek(struct.unpack_from("<I", header, 60)[0])
                pe = file.read(6)
                if pe != b"PE\0\0\x64\x86":
                    raise ValueError(f"not an amd64 PE executable: {path}")
            elif header[:5] != b"\x7fELF\x02" or header[18:20] != b"\x3e\x00":
                raise ValueError(f"not an amd64 ELF executable: {path}")
        if kind == "deb":
            audit_linux_binary(path)
        version = output([path, "--version"]).splitlines()[0].split()[-1]
        if not re.fullmatch(r"\d{8}-\d{6}-[0-9a-f]{8,40}", version):
            raise ValueError(f"invalid product version in {path}: {version}")
        if not sha.startswith(version.rsplit("-", 1)[1]):
            raise ValueError(f"stale binary {path}: {version} does not match HEAD {sha}")
        versions.add(version)
        hashes[path.name] = digest(path)
    if len(versions) != 1:
        raise ValueError("the four binaries have different product versions")
    return versions.pop(), hashes


def compile_launchers(dest: Path, kind: str):
    names = ("wezterm-gx.exe", "wezterm-gx-cli.exe") if kind == "windows" else ("wezterm-gx-gui", "wezterm-gx")
    dest.mkdir(parents=True, exist_ok=True)
    for cli, name in enumerate(names):
        cmd = [tool("rustc"), "--edition=2021", "-C", "opt-level=2"]
        if kind == "windows":
            cmd += ["-C", "target-feature=+crt-static"]
        if cli:
            cmd += ["--cfg", "gx_cli"]
        run([*cmd, ROOT / "scripts/gx-launcher/main.rs", "-o", dest / name])


def font_name(path: Path) -> str:
    """Read the first font's full name using the public SFNT name table format."""
    data = path.read_bytes()
    base = struct.unpack_from(">I", data, 12)[0] if data[:4] == b"ttcf" else 0
    count = struct.unpack_from(">H", data, base + 4)[0]
    for index in range(count):
        pos = base + 12 + index * 16
        if data[pos:pos + 4] != b"name":
            continue
        table = struct.unpack_from(">I", data, pos + 8)[0]
        _, records, strings = struct.unpack_from(">HHH", data, table)
        candidates = []
        for record in range(records):
            plat, _, lang, ident, length, offset = struct.unpack_from(">6H", data, table + 6 + record * 12)
            if ident == 4 and plat in {0, 3}:
                value = data[table + strings + offset:table + strings + offset + length].decode("utf-16-be")
                candidates.append((lang == 0x409, value))
        if candidates:
            return sorted(candidates, reverse=True)[0][1]
    raise ValueError(f"font full name missing: {path}")


def package_windows(stage: Path, dest: Path, version: str, bin_dir: Path) -> Path:
    payload = stage / "app"
    payload.mkdir()
    for name in BINARIES:
        copy_file(bin_dir / f"{name}.exe", payload / f"{name}.exe", True)
    copy_file(ROOT / "LICENSE.md", payload / "LICENSE.md")
    for source, target in WINDOWS_RUNTIME.items():
        copy_file(ROOT / "assets/windows" / source, payload / target)
    compile_launchers(payload, "windows")
    stage_resources(payload / "resources")
    font_entries = []
    for path in sorted((ROOT / "dotfiles/fonts").glob("*")):
        if path.suffix.lower() not in {".ttf", ".ttc"}:
            continue
        copy_file(path, stage / "fonts" / path.name)
        name = font_name(path).replace('"', '""')
        font_entries.append(f'Source: "{stage / "fonts" / path.name}"; DestDir: "{{autofonts}}"; '
                            f'FontInstall: "{name}"; Flags: onlyifdoesntexist uninsneveruninstall')
    (stage / "fonts.iss").write_text("\n".join(font_entries) + "\n", encoding="utf-8-sig")
    run([tool("iscc"), f"/DPackageVersion={version}", f"/DStageDir={stage}",
         f"/DRepoDir={ROOT}", f"/O{dest}", ROOT / "scripts/packaging/windows.iss"])
    return dest / f"WezTerm-GX-{version}-Setup-x64.exe"


def deb_dependencies(stage: Path, binaries: list[Path]) -> str:
    metadata = stage / "debian"
    metadata.mkdir()
    (metadata / "control").write_text(
        "Source: wezterm-gx\nSection: utils\nPriority: optional\nMaintainer: gx0404 <gx0404@users.noreply.github.com>\n\n"
        "Package: wezterm-gx\nArchitecture: amd64\nDescription: WezTerm GX terminal\n", encoding="utf-8")
    result = output([tool("dpkg-shlibdeps"), "-O", *[f"-e{p}" for p in binaries]], cwd=stage)
    dependencies = next((line.split("=", 1)[1] for line in result.splitlines() if line.startswith("shlibs:Depends=")), None)
    if not dependencies:
        raise ValueError("dpkg-shlibdeps did not produce dependencies")
    generated = set(dependencies.split(", "))
    generated_names = {item.split(" ", 1)[0] for item in generated}
    return ", ".join(sorted(generated | (set(RUNTIME_DEPS) - generated_names)))


def package_deb(stage: Path, dest: Path, version: str, bin_dir: Path, abi: dict) -> Path:
    tree = stage / "root"
    binary_root = tree / "usr/lib/wezterm-gx"
    binary_root.mkdir(parents=True)
    for name in BINARIES:
        copy_file(bin_dir / name, binary_root / name, True)
    compile_launchers(binary_root, "deb")
    stage_resources(tree / "usr/share/wezterm-gx")
    for name in ("wezterm-gx", "wezterm-gx-gui"):
        link = tree / "usr/bin" / name
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(f"../lib/wezterm-gx/{name}")
    for path in (ROOT / "dotfiles/fonts").iterdir():
        if path.suffix.lower() in {".ttf", ".ttc"}:
            copy_file(path, tree / "usr/share/fonts/truetype/wezterm-gx" / path.name)
    copy_file(ROOT / "dotfiles/assets/org.wezfurlong.wezterm.png",
              tree / "usr/share/icons/hicolor/128x128/apps/wezterm-gx.png")
    copy_file(ROOT / "scripts/packaging/wezterm-gx.desktop",
              tree / "usr/share/applications/org.gx0404.wezterm.desktop")
    copy_file(ROOT / "LICENSE.md", tree / "usr/share/doc/wezterm-gx/copyright")
    control = tree / "DEBIAN"
    control.mkdir()
    binaries = [p for p in binary_root.iterdir() if p.is_file()]
    # Audit the launchers too: --bin-dir alone does not constrain their libc.
    abi.update({path.name: audit_linux_binary(path) for path in binaries})
    dependencies = deb_dependencies(stage, binaries)
    installed_size = (sum(p.stat().st_size for p in tree.rglob("*") if p.is_file()) + 1023) // 1024
    (control / "control").write_text(
        f"Package: wezterm-gx\nVersion: {version}\nArchitecture: amd64\nSection: utils\nPriority: optional\n"
        f"Maintainer: gx0404 <gx0404@users.noreply.github.com>\nInstalled-Size: {installed_size}\n"
        f"Depends: {dependencies}\nHomepage: https://github.com/gx0404/wezterm\n"
        "Description: WezTerm GX terminal with configuration, plugins and fonts\n"
        " GPU accelerated terminal with a per-user GX environment.\n", encoding="utf-8")
    for name in ("postinst", "postrm"):
        # WSL may package a Windows checkout with CRLF working-tree files.
        (control / name).write_bytes((ROOT / "scripts/packaging" / name).read_bytes().replace(b"\r\n", b"\n"))
        (control / name).chmod(0o755)
    artifact = dest / f"wezterm-gx_{version}_amd64.deb"
    run([tool("dpkg-deb"), "-Zxz", "--root-owner-group", "--build", tree, artifact])
    run([tool("dpkg-deb"), "--info", artifact])
    return artifact


def preflight(kind: str, bin_dir: Path | None):
    if kind == "windows" and (os.name != "nt" or platform.machine().lower() not in {"amd64", "x86_64"}):
        raise ValueError("Windows packaging requires an x64 Windows build host")
    if kind == "deb":
        if sys.platform != "linux" or platform.machine() != "x86_64":
            raise ValueError("deb packaging requires x86_64 Ubuntu 20.04, or --container on Linux/WSL")
        release = Path("/etc/os-release").read_text()
        if not re.search(r'^ID=\"?ubuntu\"?$', release, re.M) or not re.search(r'^VERSION_ID="20\.04"$', release, re.M):
            raise ValueError("build deb on Ubuntu 20.04; use --container on newer Linux/WSL hosts")
    needed = ["rustc", "iscc"] if kind == "windows" else ["rustc", "dpkg-deb", "dpkg-shlibdeps", "objdump"]
    if bin_dir is None:
        needed.append("cargo")
    for name in needed:
        print(f"FOUND {name}: {tool(name)}")
    if bin_dir is None:
        output([tool("cargo"), "--version"])
    rust = output([tool("rustc"), "--version"])
    match = re.match(r"rustc (\d+)\.(\d+)", rust)
    if not match or tuple(map(int, match.groups())) < (1, 89):
        raise ValueError("the dependency-free GX launcher requires Rust >= 1.89")
    for path in (ROOT / "dotfiles/wezterm-config/wezterm.lua", ROOT / "LICENSE.md"):
        if not path.is_file():
            raise ValueError(f"missing payload: {path}")
    plugins = list((ROOT / "dotfiles/plugins").iterdir())
    if len(plugins) != 4 or any(not (p / "gitdir/HEAD").is_file()
                               or not (p / "plugin/init.lua").is_file() for p in plugins):
        raise ValueError("expected four complete GX plugin snapshots (gitdir/HEAD and plugin/init.lua)")
    if len(list((ROOT / "dotfiles/fonts").glob("*.tt*"))) < 8:
        raise ValueError("the GX font snapshot is incomplete")
    if kind == "windows":
        for name in WINDOWS_RUNTIME:
            if not (ROOT / "assets/windows" / name).is_file():
                raise ValueError(f"missing Windows runtime: {name}")


def container_deb(args, version: str):
    """Use the same native packager inside the oldest supported Ubuntu."""
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise ValueError("--container requires an amd64 Linux Docker host (including WSL)")
    docker = tool("docker")
    if output([docker, "info", "--format", "{{.OSType}}"]) != "linux":
        raise ValueError("Docker must be running Linux containers")
    if args.check:
        print("PASS: Linux Docker is available; the Ubuntu 20.04 image is provisioned only when building")
        return
    image = "wezterm-gx-deb-builder:focal"
    with tempfile.TemporaryDirectory(prefix="gx-deb-builder-") as temp:
        context = Path(temp)
        for source, name in ((ROOT / "get-deps", "get-deps"),
                             (ROOT / "ci/check-rust-version.sh", "check-rust-version.sh"),
                             (ROOT / "scripts/packaging/ubuntu2004.Dockerfile", "Dockerfile")):
            (context / name).write_bytes(source.read_bytes().replace(b"\r\n", b"\n"))
            (context / name).chmod(0o644 if name == "Dockerfile" else 0o755)
        run([docker, "build", "--build-arg", f"RUST_TOOLCHAIN={RUST_VERSION}", "--tag", image, context])
    dest = args.output_dir.resolve()
    dest.mkdir(parents=True, exist_ok=True)
    common = (ROOT / output(["git", "-C", ROOT, "rev-parse", "--git-common-dir"])).resolve()
    mounts = [ROOT, dest]
    if common != ROOT and ROOT not in common.parents:
        mounts.append(common)
    if args.bin_dir:
        mounts.append(args.bin_dir.resolve())
    cmd = [docker, "run", "--rm", "--workdir", ROOT]
    for path in dict.fromkeys(mounts):
        cmd += ["--volume", f"{path}:{path}"]
    for name, path in (("registry", "/usr/local/cargo/registry"),
                       ("git", "/usr/local/cargo/git"), ("target", "/gx-target")):
        cache = f"wezterm-gx-focal-{name}"
        if args.cache_dir:
            directory = args.cache_dir.resolve() / name
            directory.mkdir(parents=True, exist_ok=True)
            cache = str(directory)
        cmd += ["--volume", f"{cache}:{path}"]
    for name in ("CARGO_BUILD_JOBS", "CARGO_INCREMENTAL"):
        if name in os.environ:
            cmd += ["--env", f"{name}={os.environ[name]}"]
    cmd += ["--env", "CARGO_TARGET_DIR=/gx-target", "--env", "TZ=UTC", image,
            "python3", ROOT / "scripts/gx_package.py", "deb", "--version", version,
            "--output-dir", dest]
    if args.bin_dir:
        cmd += ["--bin-dir", args.bin_dir.resolve()]
    run(cmd)
    # Only hand the three generated files back to the invoking host user.
    name = f"wezterm-gx_{version}_amd64.deb"
    run([docker, "run", "--rm", "--volume", f"{dest}:{dest}", image,
         "chown", f"{os.getuid()}:{os.getgid()}",
         *[dest / (name + suffix) for suffix in ("", ".manifest.json", ".sha256")]])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["windows", "deb", "auto"])
    parser.add_argument("--version", help="GX X.Y.Z; defaults to CHANGELOG.md")
    parser.add_argument("--bin-dir", type=Path, help="package prebuilt binaries, checking their HEAD/version")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist")
    parser.add_argument("--check", action="store_true", help="read-only preflight; never builds or installs")
    parser.add_argument("--container", action="store_true", help="build deb in Ubuntu 20.04 using Docker on Linux/WSL")
    parser.add_argument("--cache-dir", type=Path, help="host directory for reusable --container Cargo caches")
    args = parser.parse_args()
    try:
        kind = ("windows" if os.name == "nt" else "deb") if args.platform == "auto" else args.platform
        version = validate_version(args.version)
        if args.cache_dir and not args.container:
            raise ValueError("--cache-dir requires --container")
        if args.container:
            if kind != "deb":
                raise ValueError("--container is only supported for deb")
            container_deb(args, version)
            return 0
        bin_dir = args.bin_dir.resolve() if args.bin_dir else Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "release"
        preflight(kind, args.bin_dir)
        sha, dirty = source_info()
        if args.check:
            if args.bin_dir:
                verify_binaries(bin_dir, kind, sha)
            print(f"PASS preflight: {kind}, GX {version}, {sha}, dirty={dirty}")
            return 0
        if not args.bin_dir:
            # The upstream build script reads .tag. Pin UTC date/hash before
            # Cargo runs so cached builds and both operating systems agree.
            (ROOT / ".tag").write_text(product_version_from_source(sha) + "\n", encoding="ascii")
            cmd = [tool("cargo"), "build", "--locked", "--release",
                   *[v for name in BINARIES for v in ("-p", name)]]
            env = os.environ.copy()
            if kind == "deb":
                cmd += ["--features", "wezterm-ssh/vendored-openssl"]
                env["OPENSSL_CONFIG_DIR"] = "/etc/ssl"
            run(cmd, cwd=ROOT, env=env)
        product_version, hashes = verify_binaries(bin_dir, kind, sha)
        dest = args.output_dir.resolve()
        dest.mkdir(parents=True, exist_ok=True)
        # deb metadata needs POSIX modes: a WSL /mnt/c or /mnt/e checkout may
        # report every directory as 0777 regardless of chmod.
        abi = {}
        with tempfile.TemporaryDirectory(prefix="gx-package-", dir=dest if kind == "windows" else None) as temp:
            artifact = (package_windows(Path(temp), dest, version, bin_dir) if kind == "windows"
                        else package_deb(Path(temp), dest, version, bin_dir, abi))
        manifest = {"schema": 1, "package_version": version, "source_commit": sha,
                    "source_dirty": dirty, "product_version": product_version,
                    "resource_version": resource_version(), "platform": kind, "architecture": "amd64",
                    "binaries": hashes, "artifact": artifact.name, "sha256": digest(artifact)}
        if kind == "deb":
            manifest["linux_compatibility"] = {"build_ubuntu": LINUX_BASELINE,
                                               "supported_ubuntu": ["20.04", "24.04"],
                                               "openssl": "static", "elf": abi}
        metadata = artifact.with_name(artifact.name + ".manifest.json")
        metadata.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        artifact.with_name(artifact.name + ".sha256").write_text(
            f"{digest(artifact)}  {artifact.name}\n{digest(metadata)}  {metadata.name}\n", encoding="ascii")
        print(f"BUILT {artifact}\nProduct: {product_version}; source_dirty={dirty}")
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

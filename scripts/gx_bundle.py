#!/usr/bin/env python3
# fork(gx): dotfiles 快照的打包/同步工具。
# bundle —— 容器（默认 ubuntu:20.04，产出 glibc>=2.31 兼容二进制）或本机构建
#           release 四件套，组装自包含离线安装包 dist/*.tar.xz；有 Windows 构建包
#           时顺带产出 Windows 部署 zip。
# sync   —— 对比本机 ~/.config/wezterm 与插件目录同 dotfiles/ 快照的差异，
#           GX_SYNC_WRITE=1 时把本机改动收回仓库（删除只报告不执行）。
# upgrade —— 一条命令完成本机替换：容器构建 release 四件套 →
#           dotfiles/install.sh 用户级部署（配置/插件/字体随快照更新，
#           全程带时间戳备份，无需 sudo）→ 版本验证。等价
#           `make gx-bundle && 安装`，但不产出离线包。
# 领域规则见 docs/AGENT_RULES/dotfiles.md 与 development.md。

import argparse
import datetime
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DOTFILES = REPO / "dotfiles"
DIST = REPO / "dist"
TARGET_GX = "target-gx-ubuntu2004"
BINARIES = ["wezterm", "wezterm-gui", "wezterm-mux-server", "strip-ansi-escapes"]
DOCKER_IMAGE = "wezterm-gx-builder:focal"
CARGO_VOLUME = "wezterm-gx-cargo"

# 构建容器镜像：apt 依赖清单唯一真源是根 get-deps（按其要求先装 rust 再跑它）。
# 注意：不能装 sudo——get-deps 检测到 sudo 会用它包 apt-get，sudo 的 env_reset
# 会丢掉 DEBIAN_FRONTEND=noninteractive，tzdata 等包的交互提示会挂死构建。
BUILDER_DOCKERFILE = """FROM ubuntu:20.04
ENV DEBIAN_FRONTEND=noninteractive \\
    CARGO_HOME=/opt/rust/cargo RUSTUP_HOME=/opt/rust/rustup \\
    PATH=/opt/rust/cargo/bin:$PATH
RUN apt-get update \\
 && apt-get install -y --no-install-recommends \\
      ca-certificates curl git lsb-release \\
 && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \\
      --default-toolchain stable --no-modify-path
WORKDIR /gx-deps
COPY get-deps ./
COPY check-rust-version.sh ./ci/
# get-deps 只做 apt-get install，索引须先重建
RUN apt-get update && ./get-deps && rm -rf /var/lib/apt/lists/*
"""


def log(msg: str) -> None:
    print(f"==> {msg}", flush=True)


def die(msg: str) -> None:
    print(f"ERROR: {msg}", file=sys.stderr, flush=True)
    sys.exit(1)


def run(cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, **kw)


def out(cmd, **kw) -> str:
    return subprocess.run(
        cmd, check=True, capture_output=True, text=True, **kw
    ).stdout.strip()


def git_version() -> str:
    try:
        v = out(["git", "-C", str(REPO), "describe", "--tags", "--always", "--dirty"])
        return re.sub(r"[^A-Za-z0-9._-]", "-", v)
    except subprocess.CalledProcessError:
        return "unknown"


def host_glibc() -> str:
    m = re.search(r"(\d+\.\d+)", out(["ldd", "--version"]).splitlines()[0])
    return m.group(1) if m else "0"


def glibc_max(binary: Path) -> str:
    """二进制引用的最高 GLIBC 符号版本（objdump 缺失时退回 strings）。"""
    try:
        text = out(["objdump", "-T", str(binary)])
    except (FileNotFoundError, subprocess.CalledProcessError):
        text = out(["strings", str(binary)])
    vers = re.findall(r"GLIBC_(\d+\.\d+)", text)
    if not vers:
        return "0"
    return sorted(vers, key=lambda v: tuple(int(x) for x in v.split(".")))[-1]


def have_docker() -> bool:
    try:
        run(["docker", "info"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except (FileNotFoundError, subprocess.CalledProcessError):
        return False


def build_in_docker() -> Path:
    if not have_docker():
        die("docker not usable; set GX_USE_LOCAL=1 to build on the host instead "
            "(resulting binaries then require the host glibc)")
    with tempfile.TemporaryDirectory(prefix="gx-builder-ctx-") as ctx:
        ctx = Path(ctx)
        shutil.copy2(REPO / "get-deps", ctx / "get-deps")
        shutil.copy2(REPO / "ci" / "check-rust-version.sh", ctx / "check-rust-version.sh")
        (ctx / "Dockerfile").write_text(BUILDER_DOCKERFILE)
        log(f"building builder image {DOCKER_IMAGE} (first run installs apt deps, slow)")
        run(["docker", "build", "-t", DOCKER_IMAGE, str(ctx)])

    # wezterm-version/build.rs 用 libgit2 取版本，容器内 root 访问宿主仓库会被
    # ownership 校验挡掉得到空版本号；改走它支持的 .tag 通道（gitignored）。
    # touch build.rs 强制 cargo 重跑该 build script，避免旧缓存继续空版本。
    tag = out(["git", "-C", str(REPO), "show", "-s",
               "--format=%cd-%h", "--date=format:%Y%m%d-%H%M%S"])
    tag_file = REPO / ".tag"
    build_rs = REPO / "wezterm-version" / "build.rs"
    log("building release binaries inside " + DOCKER_IMAGE)
    tag_file.write_text(tag + "\n")
    os.utime(build_rs)
    try:
        run([
            "docker", "run", "--rm",
            "-v", f"{REPO}:/src", "-w", "/src",
            "-v", f"{CARGO_VOLUME}:/usr/local/cargo",
            "-e", "CARGO_HOME=/usr/local/cargo",
            "-e", f"CARGO_TARGET_DIR=/src/{TARGET_GX}",
            DOCKER_IMAGE,
            "cargo", "build", "--release",
            *[arg for b in BINARIES for arg in ("-p", b)],
        ])
    finally:
        tag_file.unlink(missing_ok=True)
    # 容器内产物归 root，交还宿主用户
    run([
        "docker", "run", "--rm", "-v", f"{REPO}:/src",
        "ubuntu:20.04", "chown", "-R", f"{os.getuid()}:{os.getgid()}",
        f"/src/{TARGET_GX}",
    ])
    return REPO / TARGET_GX / "release"


def build_local() -> Path:
    if not shutil.which("cargo"):
        die("cargo not found on host; install rustup or use the docker path")
    log("building release binaries on the host (NOT 20.04-compatible)")
    run(["cargo", "build", "--release", *[arg for b in BINARIES for arg in ("-p", b)]],
        cwd=REPO)
    return REPO / "target" / "release"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def binary_version(bin_dir: Path) -> str:
    try:
        return out([str(bin_dir / "wezterm"), "--version"]).splitlines()[0]
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "dev"


def find_windows_zip() -> Path | None:
    """Windows 构建包来源：GX_WINDOWS_ZIP > dist/ 现成包 > gh 拉取 workflow 产物。"""
    env = os.environ.get("GX_WINDOWS_ZIP")
    if env:
        p = Path(env).expanduser()
        return p if p.exists() else die(f"GX_WINDOWS_ZIP not found: {p}")
    cached = sorted(DIST.glob("wezterm-windows-*.zip")) if DIST.is_dir() else []
    if cached:
        return cached[-1]
    if shutil.which("gh"):
        try:
            import json

            runs = out(["gh", "run", "list", "-R", "gx0404/wezterm",
                        "-w", "gx-windows-build.yml", "-L", "1", "--json",
                        "databaseId,conclusion"])
            info = json.loads(runs)[0]
            if info.get("conclusion") == "success":
                dest = DIST / ".gh-download"
                if dest.exists():
                    shutil.rmtree(dest)
                log(f"downloading windows artifacts from run {info['databaseId']}")
                run(["gh", "run", "download", str(info["databaseId"]),
                     "-R", "gx0404/wezterm", "-D", str(dest)])
                zips = list(dest.rglob("*.zip"))
                if zips:
                    target = DIST / zips[0].name
                    shutil.move(str(zips[0]), target)
                    return target
        except (subprocess.CalledProcessError, IndexError, KeyError, ValueError):
            pass
    return None


def stage_common(stage: Path) -> None:
    """bundle 根目录公共部分：dotfiles 快照 + install 脚本。"""
    shutil.copytree(DOTFILES, stage / "dotfiles")
    (stage / "install.sh").write_bytes((DOTFILES / "install.sh").read_bytes())
    os.chmod(stage / "install.sh", 0o755)


def write_manifest(stage: Path, fields: dict) -> None:
    lines = [f'{k}="{v}"' for k, v in fields.items()]
    (stage / "manifest.env").write_text("\n".join(lines) + "\n")


def write_sha256sums(stage: Path) -> None:
    entries = []
    for p in sorted(stage.rglob("*")):
        if p.is_file() and p.name != "SHA256SUMS":
            entries.append(f"{sha256_file(p)}  {p.relative_to(stage)}")
    (stage / "SHA256SUMS").write_text("\n".join(entries) + "\n")


def make_tar_xz(stage_dir: Path, out_file: Path) -> None:
    """打包 staging 目录本身（成员名 = 目录名，-C 取其父目录）。"""
    log(f"packing {out_file}")
    run(["tar", "-C", str(stage_dir.parent), "-c",
         "--use-compress-program=xz -T0",
         "-f", str(out_file), stage_dir.name])


def cmd_bundle(args) -> None:
    use_local = args.use_local or os.environ.get("GX_USE_LOCAL") == "1"
    bin_dir = build_local() if use_local else build_in_docker()
    for b in BINARIES:
        if not (bin_dir / b).exists():
            die(f"missing built binary: {bin_dir / b}")

    version = binary_version(bin_dir)
    gmax = max(glibc_max(bin_dir / b) for b in BINARIES)
    glibc_min = host_glibc() if use_local else gmax
    tag = f"wezterm-gx-{git_version()}"
    suffix = f"-glibc{glibc_min}" if use_local else ""
    DIST.mkdir(exist_ok=True)

    stage = DIST / f"{tag}-linux-amd64{suffix}"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    stage_common(stage)
    (stage / "bin").mkdir()
    for b in BINARIES:
        shutil.copy2(bin_dir / b, stage / "bin" / b)
        os.chmod(stage / "bin" / b, 0o755)
    write_manifest(stage, {
        "VERSION": version,
        "COMMIT": git_version(),
        "GLIBC_MIN": glibc_min,
        "GLIBC_MAX": gmax,
        "BUILT_ON": datetime.date.today().isoformat(),
        "BUILD_ENV": "host" if use_local else "docker:ubuntu:20.04",
    })
    write_sha256sums(stage)
    make_tar_xz(stage, DIST / f"{stage.name}.tar.xz")
    print(f"linux bundle: {DIST / (stage.name + '.tar.xz')}")
    print(f"  version={version} glibc_min={glibc_min} (symbols max GLIBC_{gmax})")
    shutil.rmtree(stage)

    # ---- Windows bundle（可选）----
    win_zip = None if args.linux_only else find_windows_zip()
    if win_zip is None:
        print("windows bundle: skipped (no windows zip; see dotfiles/README.md)")
        return
    win_stage = DIST / f"{tag}-windows-x86_64"
    if win_stage.exists():
        shutil.rmtree(win_stage)
    win_stage.mkdir(parents=True)
    stage_common(win_stage)
    for tool in ("unzip", "zip"):
        if not shutil.which(tool):
            die(f"'{tool}' is required to pack the windows bundle")
    (win_stage / "bin-windows").mkdir()
    with tempfile.TemporaryDirectory() as tmp:
        run(["unzip", "-q", str(win_zip), "-d", tmp])
        root = Path(tmp)
        gui = next(root.rglob("wezterm-gui.exe"), None)
        if gui is None:
            die(f"wezterm-gui.exe not found in {win_zip}")
        for exe in gui.parent.glob("*.exe"):
            shutil.copy2(exe, win_stage / "bin-windows" / exe.name)
        win_version = "dev"
        try:
            win_version = out([str(gui.parent / "wezterm.exe"), "--version"]).splitlines()[0]
        except (subprocess.CalledProcessError, FileNotFoundError):
            pass
    write_manifest(win_stage, {
        "VERSION": win_version,
        "COMMIT": git_version(),
        "BUILT_ON": datetime.date.today().isoformat(),
        "BUILD_ENV": "github-actions:windows",
    })
    write_sha256sums(win_stage)
    win_out = DIST / f"{win_stage.name}.zip"
    log(f"packing {win_out}")
    run(["zip", "-q", "-r", str(win_out), win_stage.name], cwd=DIST)
    print(f"windows bundle: {win_out}")
    shutil.rmtree(win_stage)


# ------------------------------------------------------------------- sync ----

SYNC_IGNORE_DIRS = {".git", "gitdir", "state"}  # gitdir=插件 .git 快照; state=resurrect 会话数据
PRINT_LIMIT = 50  # 每类最多打印条数


def walk_files(root: Path):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SYNC_IGNORE_DIRS]
        for name in filenames:
            p = Path(dirpath) / name
            yield p.relative_to(root), p


def diff_tree(live: Path, snap: Path, label: str):
    if not live.exists():
        print(f"[{label}] live dir missing: {live}")
        return [], [], []
    live_files = {rel: p for rel, p in walk_files(live)}
    snap_files = {rel: p for rel, p in walk_files(snap)}
    added, changed, removed = [], [], []
    for rel, p in live_files.items():
        if rel not in snap_files:
            added.append(rel)
        elif sha256_file(p) != sha256_file(snap_files[rel]):
            changed.append(rel)
    for rel in snap_files:
        if rel not in live_files:
            removed.append(rel)
    for title, items in (("ADDED (machine only)", added),
                         ("CHANGED", changed),
                         ("REMOVED (repo only, never auto-deleted)", removed)):
        if items:
            print(f"[{label}] {title}: {len(items)}")
            for rel in sorted(items)[:PRINT_LIMIT]:
                print(f"    {rel}")
            if len(items) > PRINT_LIMIT:
                print(f"    ... and {len(items) - PRINT_LIMIT} more")
    return added, changed, removed


def cmd_sync(args) -> None:
    write = args.write or os.environ.get("GX_SYNC_WRITE") == "1"
    pairs = [
        (Path.home() / ".config" / "wezterm", DOTFILES / "wezterm-config", "config"),
        (Path.home() / ".local" / "share" / "wezterm" / "plugins",
         DOTFILES / "plugins", "plugins"),
    ]
    total = 0
    for live, snap, label in pairs:
        added, changed, _ = diff_tree(live, snap, label)
        total += len(added) + len(changed)
        if write:
            for rel in added + changed:
                src = live / rel
                dst = snap / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)
    # 已知的收录期有意改动（PROVENANCE.md 记录）不算意外差异
    known = ["config/launch.lua", "config/domains.lua"]
    known_hits = [k for k in known
                  if (DOTFILES / "wezterm-config" / k).exists()]
    if known_hits:
        print(f"note: {', '.join(known_hits)} intentionally differ from the "
              f"machine copy (see dotfiles/PROVENANCE.md)")
    if total == 0:
        print("sync: no differences")
    elif write:
        print(f"sync: copied {total} file(s) back into dotfiles/")
    else:
        print(f"sync: {total} difference(s); run with GX_SYNC_WRITE=1 to sync back")


def cmd_upgrade(args) -> None:
    use_local = args.use_local or os.environ.get("GX_USE_LOCAL") == "1"
    bin_dir = build_local() if use_local else build_in_docker()
    for b in BINARIES:
        if not (bin_dir / b).exists():
            die(f"missing built binary: {bin_dir / b}")

    run(["bash", str(DOTFILES / "install.sh"),
         "--from-build", str(bin_dir), "--bundle-root", str(REPO)])

    wrapper = Path.home() / ".local" / "bin" / "wezterm"
    version = out([str(wrapper), "--version"])
    print(f"upgrade verified: {version}")
    print("note: 已打开的 wezterm 窗口仍运行旧二进制；重启 wezterm"
          "（退出后从桌面/命令行重新启动）后新版本生效。"
          "回滚：~/.local/bin 下 .bak-gx-* 备份与 ~/.local/opt/wezterm-nightly。")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("bundle", help="build + pack offline installers")
    b.add_argument("--use-local", action="store_true",
                   help="build on the host instead of the ubuntu:20.04 container")
    b.add_argument("--linux-only", action="store_true", help="skip windows bundle")
    s = sub.add_parser("sync", help="diff/copy machine state back into dotfiles/")
    s.add_argument("--write", action="store_true", help="copy changes back")
    u = sub.add_parser("upgrade",
                       help="build in container + user-level install + verify")
    u.add_argument("--use-local", action="store_true",
                   help="build on the host instead of the ubuntu:20.04 container")
    args = ap.parse_args()
    if args.cmd == "bundle":
        cmd_bundle(args)
    elif args.cmd == "upgrade":
        cmd_upgrade(args)
    else:
        cmd_sync(args)


if __name__ == "__main__":
    main()

.PHONY: all fmt build check test docs servedocs

all: build

test:
	cargo nextest run
	cargo nextest run -p wezterm-escape-parser # no_std by default

check:
	cargo check
	cargo check -p wezterm-escape-parser
	cargo check -p wezterm-cell
	cargo check -p wezterm-surface
	cargo check -p wezterm-ssh

build:
	cargo build $(BUILD_OPTS) -p wezterm
	cargo build $(BUILD_OPTS) -p wezterm-gui
	cargo build $(BUILD_OPTS) -p wezterm-mux-server
	cargo build $(BUILD_OPTS) -p strip-ansi-escapes

fmt:
	cargo +nightly fmt

docs:
	ci/build-docs.sh

servedocs:
	ci/build-docs.sh serve

# ---------------------------------------------------------------------------
# AI 协作开发框架（fork 维护段，上游没有；命令手册见 docs/MAKE_COMMANDS.md）
# 工具解析序：项目钉版 .local/tools > 系统 PATH（安装：make setup）。
TOOLS_ROOT := $(subst \,/,$(or $(WEZTERM_TOOLCHAIN_ROOT),$(CURDIR)/.local/tools))
PATH_SEPARATOR := :
ifneq ($(filter %mingw32 %windows32 %windows-gnu,$(MAKE_HOST)),)
PATH_SEPARATOR := ;
# Git 的 bin/sh 启动器会把自带 Perl 重新前置；直接使用实际 shell。
SHELL := $(subst \,/,$(or $(MAKESHELL),$(ProgramFiles)/Git/usr/bin/sh.exe))
endif
export PATH := $(TOOLS_ROOT)/make/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/venv/Scripts$(PATH_SEPARATOR)$(TOOLS_ROOT)/venv/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/nextest/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/stylua/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/lua/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/nasm/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/perl/perl/bin$(PATH_SEPARATOR)$(TOOLS_ROOT)/perl/c/bin$(PATH_SEPARATOR)$(PATH)
FRAMEWORK_PY := $(or $(wildcard $(TOOLS_ROOT)/venv/Scripts/python.exe),$(wildcard $(TOOLS_ROOT)/venv/bin/python),python3)
ifeq ($(OS),Windows_NT)
# GNU make 的 jobserver/命令行选项不是 OpenSSL 子进程 NMake 的选项。
unexport MAKEFLAGS MFLAGS GNUMAKEFLAGS
endif

# 日常二进制必须是优化构建：上游 build 目标的 $(BUILD_OPTS) 未定义时 cargo
# 落 dev profile（opt-level 0 + debug assertions），高速输出/滚动明显卡顿。
# 需要开发期快速迭代时显式 BUILD_OPTS= make build 覆盖。
BUILD_OPTS ?= --release

# 编译、构建的一切产物只落在仓库目录内（规则见 docs/AGENT_RULES/development.md
# 「构建产物仓内封闭」）：构建目标 target/、sccache 编译缓存、构建进程临时文件
# 都固定仓内，防止外部环境变量或工具默认值把产物带到仓外。.local/ 已整目录
# gitignore；产物目录不存在时这里就地创建。
export CARGO_TARGET_DIR := $(CURDIR)/target
SCCACHE_CACHE_DIR := $(CURDIR)/.local/sccache
export SCCACHE_DIR := $(SCCACHE_CACHE_DIR)
BUILD_TMP_DIR := $(CURDIR)/.local/tmp
export TMP := $(BUILD_TMP_DIR)
export TEMP := $(BUILD_TMP_DIR)
export TMPDIR := $(BUILD_TMP_DIR)
$(shell "$(FRAMEWORK_PY)" -c "from pathlib import Path; Path('$(SCCACHE_CACHE_DIR)').mkdir(parents=True, exist_ok=True); Path('$(BUILD_TMP_DIR)').mkdir(parents=True, exist_ok=True)")

# Git Bash 调 MSYS2 make 时两套运行时可能丢失 Windows 已知文件夹变量。
ifneq ($(filter %-cygwin %-msys,$(MAKE_HOST)),)
ifeq ($(USERPROFILE),)
WIN_PROFILE_DIR := $(shell /usr/bin/cygpath -w -F 40)
ifneq ($(WIN_PROFILE_DIR),)
export USERPROFILE := $(WIN_PROFILE_DIR)
endif
endif
ifeq ($(LOCALAPPDATA),)
WIN_LOCAL_APPDATA_DIR := $(shell /usr/bin/cygpath -w -F 28)
ifneq ($(WIN_LOCAL_APPDATA_DIR),)
export LOCALAPPDATA := $(WIN_LOCAL_APPDATA_DIR)
endif
endif
endif

# build/test 沿用上方上游目标语义；框架命令经 dev_framework.py 调度，不重复定义。
FRAMEWORK_COMMANDS := setup dev lint typecheck test-integration test-heavy \
	generated-check generated-write ui-smoke graph graph-check kb kb-check framework-test \
	gx-bundle gx-install gx-sync gx-upgrade package gx-package-windows gx-package-deb

.PHONY: help framework-check framework-ready ai-doctor ci-check version version-check version-write evidence $(FRAMEWORK_COMMANDS)

help:
	@echo "上游目标: all build check test fmt docs servedocs  (语义见 CONTRIBUTING.md)"
	@$(FRAMEWORK_PY) scripts/dev_framework.py help

framework-check:
	$(FRAMEWORK_PY) scripts/dev_framework.py check

framework-ready:
	$(FRAMEWORK_PY) scripts/dev_framework.py ready

ai-doctor:
	$(FRAMEWORK_PY) scripts/dev_framework.py doctor

ci-check:
	$(FRAMEWORK_PY) scripts/dev_framework.py ci

version:
	$(FRAMEWORK_PY) scripts/version.py

version-check:
	$(FRAMEWORK_PY) scripts/version.py --check

version-write:
	$(FRAMEWORK_PY) scripts/version.py --write

TASK ?= ui-smoke
evidence:
	$(FRAMEWORK_PY) scripts/dev_framework.py evidence $(TASK)

$(FRAMEWORK_COMMANDS):
	$(FRAMEWORK_PY) scripts/dev_framework.py run $@

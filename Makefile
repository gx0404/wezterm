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
export PATH := $(CURDIR)/.local/tools/venv/bin:$(CURDIR)/.local/tools/nextest/bin:$(CURDIR)/.local/tools/stylua/bin:$(CURDIR)/.local/tools/lua/bin:$(PATH)

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
$(shell mkdir -p "$(SCCACHE_CACHE_DIR)" "$(BUILD_TMP_DIR)")
FRAMEWORK_PY := $(if $(wildcard .local/tools/venv/bin/python),.local/tools/venv/bin/python,python3)

# Git Bash 调 MSYS2 make 时两套 msys-2.0.dll 运行时互不相认，子进程环境只剩 PATH/SYSTEMROOT
# 等少数变量：缺 TMP/TEMP 时 dlltool/gcc 回退到 C:\WINDOWS\ 建临时文件而失败，缺 USERPROFILE
# 时 Python 的 Path.home() 抛 RuntimeError，缺 LOCALAPPDATA 时 gx_package.py 找不到用户级
# Inno Setup。仅 cygwin/msys 版 make 下补缺失项，不覆盖已有值：TMP/TEMP 优先仓内
# .local/tmp（构建临时文件不外泄），USERPROFILE/LOCALAPPDATA 仍取 Windows 已知文件夹。
ifneq ($(filter %-cygwin %-msys,$(MAKE_HOST)),)
ifeq ($(and $(TMP),$(TEMP)),)
export TMP := $(or $(TMP),$(BUILD_TMP_DIR))
export TEMP := $(or $(TEMP),$(BUILD_TMP_DIR))
endif
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

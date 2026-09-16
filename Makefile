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
export PATH := $(CURDIR)/.local/tools/venv/bin:$(CURDIR)/.local/tools/nextest/bin:$(PATH)
FRAMEWORK_PY := $(if $(wildcard .local/tools/venv/bin/python),.local/tools/venv/bin/python,python3)

# build/test 沿用上方上游目标语义；框架命令经 dev_framework.py 调度，不重复定义。
FRAMEWORK_COMMANDS := setup dev lint typecheck test-integration test-heavy \
	generated-check generated-write ui-smoke graph graph-check kb kb-check framework-test

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

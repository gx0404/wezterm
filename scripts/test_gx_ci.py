"""GX CI text contracts and executable version-reader tests; YAML validation needs actionlint."""
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/gx-ci.yml"


class WorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def job(self, name):
        match = re.search(rf"^  {name}:\n(.*?)(?=^  [\w-]+:|\Z)",
                          self.workflow.split("\njobs:\n", 1)[1], re.M | re.S)
        self.assertIsNotNone(match, name)
        return match.group(1)

    def test_only_gx_push_pull_request_and_manual_events(self):
        events = self.workflow.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
        self.assertEqual(textwrap.dedent(events).strip(), textwrap.dedent("""\
            push:
              branches: [feature/gx_wezterm]
            pull_request:
              branches: [feature/gx_wezterm]
            workflow_dispatch:
        """).strip())

    def test_read_only_and_no_publishing_or_privileged_pr_context(self):
        self.assertEqual(re.findall(r"^permissions:\n((?:  .+\n)+)", self.workflow, re.M),
                         ["  contents: read\n"])
        self.assertNotRegex(self.workflow, r"(?m)^ +permissions:")
        for forbidden in ("pull_request_target", "secrets.", "contents: write",
                          "ci/deploy.sh", "ci/create-release.sh", "ci/tag.sh",
                          "gx_release.py", "gx_package.py windows", "gx_package.py deb",
                          "gh release", "git push", "continue-on-error:"):
            self.assertNotIn(forbidden, self.workflow)
        checkouts = re.findall(r"- uses: actions/checkout@[^\n]+\n((?:        .+\n)+)",
                               self.workflow)
        self.assertEqual(len(checkouts), 3)
        for checkout in checkouts:
            self.assertIn("persist-credentials: false", checkout)
            self.assertNotIn("ref:", checkout)

    def test_concurrency_cancels_same_pr_or_ref_without_touching_release(self):
        concurrency = self.workflow.split("\nconcurrency:\n", 1)[1].split("\nenv:", 1)[0]
        self.assertIn("group: gx-ci-${{ github.event.pull_request.number || github.ref }}", concurrency)
        self.assertIn("cancel-in-progress: true", concurrency)
        self.assertNotIn("gx-release", concurrency)

    def test_fast_checks_gate_both_platforms(self):
        checks = self.job("checks")
        for command in ("python3 scripts/resolve_agent_rules.py --check",
                        "python3 scripts/dev_framework.py ready",
                        "python3 scripts/version.py --check",
                        "python3 -m unittest discover -s scripts -p 'test_*.py'",
                        "cargo +nightly fmt --all --check",
                        "rustfmt +nightly --edition 2021 --check scripts/gx-launcher/main.rs"):
            self.assertIn(command, checks)
        self.assertIn("dtolnay/rust-toolchain@nightly", checks)
        for platform in ("linux", "windows"):
            self.assertIn("needs: checks", self.job(platform))

    def test_platforms_have_recursive_checkout_and_bounded_resources(self):
        self.assertIn("CARGO_INCREMENTAL: '0'", self.workflow)
        self.assertIn("CARGO_BUILD_JOBS: '2'", self.workflow)
        for name, runner in (("checks", "ubuntu-24.04"), ("linux", "ubuntu-24.04"),
                             ("windows", "windows-2025")):
            job = self.job(name)
            self.assertIn(f"runs-on: {runner}", job)
            timeout = re.search(r"timeout-minutes: (\d+)", job)
            self.assertIsNotNone(timeout)
            self.assertLessEqual(int(timeout.group(1)), 120)
            if name != "checks":
                self.assertIn("submodules: recursive", job)
                self.assertIn("toolchain: ${{ needs.checks.outputs.rust }}", job)

    def test_platform_cache_keys_include_pins_and_lockfile(self):
        for platform in ("linux", "windows"):
            job = self.job(platform)
            self.assertIn("uses: actions/cache@", job)
            key = re.search(r"          key: (.+)", job).group(1)
            self.assertTrue(key.startswith(f"gx-ci-{platform}-"))
            for part in ("runner.arch", "needs.checks.outputs.rust", "needs.checks.outputs.nextest",
                         "hashFiles('**/Cargo.lock'", "github.sha"):
                self.assertIn(part, key)
            self.assertIn(".local/tools/nextest", job)

    def test_nextest_install_uses_shared_pin_without_running_setup(self):
        for platform, variable in (("linux", "$NEXTEST_VERSION"),
                                   ("windows", "$env:NEXTEST_VERSION")):
            job = self.job(platform)
            self.assertIn("NEXTEST_VERSION: ${{ needs.checks.outputs.nextest }}", job)
            self.assertIn(f'cargo install cargo-nextest --version "={variable}" --locked --root .local/tools/nextest', job)
            self.assertIn("GITHUB_PATH", job)
        self.assertNotRegex(self.workflow, r"(?:bash|source|\.) scripts/setup_env\.sh")
        self.assertNotIn("graphify", self.workflow)
        self.assertNotRegex(self.workflow, r"toolchain: ['\"]?\d+\.\d+")

    def test_linux_prepares_ssh_integration_dependencies_before_testing(self):
        linux = self.job("linux")
        commands = ("sudo apt-get update", "env CI=yes ./get-deps --testing",
                    "sudo apt-get install -y openssh-client lua5.4", "test -x /usr/sbin/sshd",
                    "command -v ssh-agent", "command -v ssh-keygen", "sudo mkdir -p /run/sshd")
        positions = [linux.index(command) for command in commands]
        self.assertEqual(positions, sorted(positions))
        self.assertLess(positions[-1], linux.index("cargo nextest run"))
        self.assertIn("run: make check", linux)
        self.assertIn("run: make build BUILD_OPTS=--locked", linux)
        self.assertIn("lua5.4 scripts/tests/gx_resurrect.lua", linux)

    def test_windows_matches_make_check_and_build_targets(self):
        windows = self.job("windows")
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        check_targets = re.findall(r"^\tcargo check -p ([\w-]+)$", makefile, re.M)
        build_targets = re.findall(r"^\tcargo build \$\(BUILD_OPTS\) -p ([\w-]+)$", makefile, re.M)
        self.assertTrue(check_targets)
        self.assertEqual(len(build_targets), 4)
        for targets in (check_targets, build_targets):
            self.assertIn("@(" + ", ".join(repr(target) for target in targets) + ")", windows)
        for command in ("cargo check --locked", "cargo check --locked -p $crate",
                        "cargo build --locked -p $crate"):
            self.assertIn(command, windows)
        self.assertIn("C:\\Strawberry\\perl\\bin", windows)
        self.assertIn('Test-Path "$perl\\perl.exe"', windows)
        self.assertIn("ilammy/msvc-dev-cmd@v1", windows)
        self.assertIn("target: x86_64-pc-windows-msvc", windows)
        self.assertIn("from test_windows_resources import find_windows_sdk; find_windows_sdk()", windows)
        self.assertIn("python -m unittest discover -s scripts -p test_windows_resources.py -v", windows)

    def test_every_windows_native_command_propagates_failure_immediately(self):
        lines = self.job("windows").splitlines()
        native_commands = []
        for index, line in enumerate(lines):
            if re.match(r"\s*(?:cargo |rustc |git |python |& \.local/)", line):
                native_commands.append(line)
                self.assertEqual(lines[index + 1].strip(),
                                 "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }", line)
        self.assertGreaterEqual(len(native_commands), 9)
        self.assertIn("shell: pwsh", self.job("windows"))

    def test_workspace_and_standalone_no_std_tests_run_on_both_platforms(self):
        for platform in ("linux", "windows"):
            job = self.job(platform)
            self.assertIn("cargo nextest run --locked --all --no-fail-fast --test-threads 2", job)
            self.assertIn("cargo nextest run --locked -p wezterm-escape-parser --no-fail-fast --test-threads 2", job)
            self.assertNotIn("--all-features", job)
            self.assertNotIn("--features", job)
            self.assertIn("rustc --edition=2021 --test scripts/gx-launcher/main.rs", job)
            self.assertIn(".local/gx-tests/launcher-tests", job)


class VersionReaderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        body = workflow.split("          python3 - <<'PY'\n", 1)[1].split("          PY\n", 1)[0]
        cls.code = textwrap.dedent(body)

    def run_reader(self, root):
        output = root / "output"
        result = subprocess.run([sys.executable, "-B", "-c", self.code], cwd=root,
                                env=dict(os.environ, GITHUB_OUTPUT=str(output)),
                                capture_output=True, text=True)
        content = output.read_text(encoding="utf-8") if output.exists() else ""
        return result, content

    def test_reads_current_repository_authorities(self):
        import gx_package

        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            result = subprocess.run([sys.executable, "-B", "-c", self.code], cwd=ROOT,
                                    env=dict(os.environ, GITHUB_OUTPUT=str(output)),
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            expected = re.search(r'^NEXTEST_VERSION="([^"]+)"$',
                                 (ROOT / "scripts/setup_env.sh").read_text(encoding="utf-8"), re.M).group(1)
            self.assertEqual(output.read_text(encoding="utf-8"),
                             f"rust={gx_package.RUST_VERSION}\nnextest={expected}\n")

    def test_source_version_changes_flow_to_outputs_without_workflow_edits(self):
        for rust, nextest in (("1.2.3", "0.4.5"), ("9.8.7", "6.5.4")):
            with self.subTest(rust=rust), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "scripts").mkdir()
                (root / "scripts/gx_package.py").write_text(f'RUST_VERSION = "{rust}"\n')
                (root / "scripts/setup_env.sh").write_text(f'NEXTEST_VERSION="{nextest}"\n')
                result, content = self.run_reader(root)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(content, f"rust={rust}\nnextest={nextest}\n")

    def test_rejects_missing_duplicate_or_malformed_pins_without_outputs(self):
        for rust, declaration in (("1.2.3", ""), ("1.2.3", 'NEXTEST_VERSION="oops"\n'),
                                  ("1.2.3", 'NEXTEST_VERSION="0.4.5"\n' * 2),
                                  ("stable", 'NEXTEST_VERSION="0.4.5"\n')):
            with self.subTest(rust=rust, declaration=declaration), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "scripts").mkdir()
                (root / "scripts/gx_package.py").write_text(f'RUST_VERSION = "{rust}"\n')
                (root / "scripts/setup_env.sh").write_text(declaration)
                result, content = self.run_reader(root)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Expected one pinned Rust and nextest version", result.stderr)
                self.assertEqual(content, "")


if __name__ == "__main__":
    unittest.main()

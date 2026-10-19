# Copyright 2024 Stellar-K8s Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exercise the shared cargo installer without network access or real sleeps."""

import os
from pathlib import Path
import shutil
import subprocess
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[2]


def installer_script():
    source = (ROOT / ".github/actions/setup-rust/action.yml").read_text(encoding="utf-8")
    step = source.split("    - name: Install extra cargo tools\n", 1)[1]
    body = step.split("      run: |\n", 1)[1]
    lines = []
    for line in body.splitlines():
        if line and not line.startswith("        "):
            break
        lines.append(line)
    return textwrap.dedent("\n".join(lines))


class CargoInstallerTests(unittest.TestCase):
    def run_installer(self, mock, suffix="", tools="fake-cargo-tool"):
        bash = shutil.which("bash")
        self.assertIsNotNone(bash, "Bash is required for installer regression coverage")
        return subprocess.run(
            [bash, "--noprofile", "--norc", "-c", mock + "\n" + installer_script() + "\n" + suffix],
            cwd=ROOT,
            env={**os.environ, "EXTRA_TOOLS": tools},
            text=True,
            encoding="utf-8",
            capture_output=True,
            timeout=10,
        )

    def test_recovers_on_third_attempt_with_increasing_delays(self):
        result = self.run_installer(
            'attempts=0; sleeps=""; '
            'cargo() { attempts=$((attempts+1)); [[ "$attempts" -ge 3 ]]; }; '
            'sleep() { sleeps+="$1,"; };',
            '[[ "$attempts" == 3 && "$sleeps" == "5,10," ]]',
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_permanent_failure_remains_fatal_after_three_attempts(self):
        result = self.run_installer(
            'cargo() { echo INSTALL_ATTEMPT; return 1; }; sleep() { :; };'
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count("INSTALL_ATTEMPT"), 3)
        self.assertIn("Failed to install fake-cargo-tool after 3 attempts", result.stdout)

    def test_cached_tool_skips_cargo(self):
        result = self.run_installer(
            'fake-cargo-tool() { :; }; cargo() { echo UNEXPECTED_INSTALL; return 1; };'
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("UNEXPECTED_INSTALL", result.stdout)


if __name__ == "__main__":
    unittest.main()

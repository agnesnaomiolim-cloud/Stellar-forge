"""Portable path and content-preservation contracts for the license fixer."""

import importlib.util
import tempfile
import unittest
from pathlib import Path, PureWindowsPath
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "license_gate", Path(__file__).resolve().parents[1] / "check-license-headers.py"
)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class LicenseHeadersTest(unittest.TestCase):
    def test_windows_exclusions_use_repository_paths(self):
        root = PureWindowsPath("C:/repository")
        with patch.object(gate, "REPO_ROOT", root):
            self.assertTrue(gate.should_exclude(root / "examples" / "node.yaml"))
            self.assertTrue(gate.should_exclude(root / ".tools" / "rendered.yaml"))
            self.assertFalse(gate.should_exclude(root / "src" / "lib.rs"))

    def test_fix_preserves_body_and_final_newline(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "script.sh"
            body = '#!/usr/bin/env bash\necho "hello"\n'
            path.write_text(body, encoding="utf-8")
            gate.fix_header(path, gate.SHELL_HEADER)
            self.assertEqual(
                path.read_text(encoding="utf-8"),
                "#!/usr/bin/env bash\n" + gate.SHELL_HEADER + 'echo "hello"\n',
            )
            self.assertTrue(gate.has_valid_header(path, gate.SHELL_HEADER)[0])

    def test_fix_does_not_invent_a_final_newline(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lib.rs"
            path.write_text("fn main() {}", encoding="utf-8")
            gate.fix_header(path, gate.RUST_HEADER)
            self.assertEqual(path.read_text(encoding="utf-8"), gate.RUST_HEADER + "fn main() {}")

    def test_rust_spdx_is_an_accepted_license(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lib.rs"
            path.write_text("// SPDX-License-Identifier: Apache-2.0\n", encoding="utf-8")
            self.assertTrue(gate.has_valid_header(path, gate.RUST_HEADER)[0])

    def test_fix_does_not_move_a_bom_inside_rust_source(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lib.rs"
            path.write_text("fn main() {}\n", encoding="utf-8-sig")
            gate.fix_header(path, gate.RUST_HEADER)
            self.assertEqual(path.read_text(encoding="utf-8"), gate.RUST_HEADER + "fn main() {}\n")


if __name__ == "__main__":
    unittest.main()

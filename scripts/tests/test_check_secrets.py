"""The secret scan distinguishes format markers from actual PEM boundaries."""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCANNER = Path(__file__).resolve().parents[1] / "check-secrets.sh"
BASH = os.environ.get("BASH_EXE") or shutil.which("bash")


@unittest.skipUnless(BASH, "bash is required for scanner integration tests")
class SecretScannerTest(unittest.TestCase):
    def scan(self, source):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("src", "scripts", ".github/workflows"):
                (root / name).mkdir(parents=True, exist_ok=True)
            shutil.copyfile(SCANNER, root / "scripts/check-secrets.sh")
            (root / "src/app.rs").write_text(source, encoding="utf-8")
            return subprocess.run(
                [BASH, "scripts/check-secrets.sh"], cwd=root,
                capture_output=True, text=True, encoding="utf-8", timeout=30,
            )

    def test_pem_format_assertions_are_not_credentials(self):
        result = self.scan('assert!(generated_key.contains("BEGIN PRIVATE KEY"));\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_full_pem_boundary_is_rejected(self):
        # A delimiter alone is enough to trigger the gate; no key material is
        # created or committed by this test.
        boundary = "-----" + "BEGIN PRIVATE KEY" + "-----"
        result = self.scan('let value = "' + boundary + '\\n";\n')
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("PEM private key block", result.stdout)

    def test_private_key_variants_are_rejected(self):
        for kind in ("RSA ", "EC ", "OPENSSH ", "ENCRYPTED "):
            with self.subTest(kind=kind):
                boundary = "-----BEGIN " + kind + "PRIVATE KEY-----"
                result = self.scan('let value = "' + boundary + '\\n";\n')
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_full_length_stellar_seed_candidate_is_rejected(self):
        # Pattern-shaped data generated at runtime; this is not a usable key.
        candidate = "S" + "A" * 55
        result = self.scan('let candidate = "' + candidate + '";\n')
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("Possible Stellar seed", result.stdout)

    def test_short_base32_string_is_not_a_stellar_seed(self):
        candidate = "S" + "A" * 54
        result = self.scan('let candidate = "' + candidate + '";\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

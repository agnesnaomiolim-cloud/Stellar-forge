#!/usr/bin/env python3
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Regression coverage for generated manifest formatting and scalar fidelity."""

from pathlib import Path
import subprocess
import sys
import unittest

import yaml


SORTER = Path(__file__).resolve().parents[1] / "sort-manifests.py"


def render(source):
    return subprocess.check_output(
        [sys.executable, str(SORTER)], input=source, text=True, encoding="utf-8"
    )


class ManifestSorterTest(unittest.TestCase):
    def test_multiline_configuration_and_shell_scripts_are_preserved(self):
        values = {
            "script": "#!/bin/sh\nprintf '%s\\n' 'value with spaces'\n",
            "description": "Independent historical baseline and operational metric comparison. " * 5 + "\n",
            "empty": "",
            "two-newlines": "value\n\n",
            "trailing-space": "value \nnext value\n",
        }
        source = yaml.safe_dump({"kind": "ConfigMap", "metadata": {"name": "test"}, "data": values})
        result = render(source)
        self.assertEqual(yaml.safe_load(result)["data"], values)
        self.assertIn("script: |", result)
        self.assertEqual(render(result), result)

    def test_document_and_mapping_sorting_is_stable(self):
        source = "kind: Service\nmetadata: {name: z}\n---\nkind: ConfigMap\nmetadata: {name: a}\n---\n"
        result = render(source)
        self.assertEqual([doc["kind"] for doc in yaml.safe_load_all(result)], ["ConfigMap", "Service"])
        self.assertEqual(render(result), result)


if __name__ == "__main__":
    unittest.main()

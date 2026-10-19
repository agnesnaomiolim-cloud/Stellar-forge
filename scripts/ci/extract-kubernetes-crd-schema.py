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

"""Generate the CRD object schema missing from kubeconform's default catalog.

Uses the official Kubernetes v1.30.0 OpenAPI document. Reproduce with:
python scripts/ci/extract-kubernetes-crd-schema.py --source <downloaded-openapi.json>
"""

import argparse
import json
from pathlib import Path
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parents[2]
SOURCE = (
    "https://raw.githubusercontent.com/kubernetes/kubernetes/v1.30.0/"
    "api/openapi-spec/v3/apis__apiextensions.k8s.io__v1_openapi.json"
)
TARGET = ROOT / "schemas/kubernetes/CustomResourceDefinition.json"


def convert(value):
    if isinstance(value, list):
        return [convert(item) for item in value]
    if not isinstance(value, dict):
        return value
    result = {key: convert(item) for key, item in value.items()}
    if isinstance(result.get("$ref"), str):
        result["$ref"] = result["$ref"].replace("#/components/schemas/", "#/definitions/")
    if result.get("type") == "object" and "properties" in result:
        result.setdefault("additionalProperties", False)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    if args.source:
        document = json.loads(args.source.read_text(encoding="utf-8"))
    else:
        with urlopen(SOURCE, timeout=30) as response:
            document = json.load(response)
    definitions = document["components"]["schemas"]
    schema = {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$ref": "#/definitions/io.k8s.apiextensions-apiserver.pkg.apis.apiextensions.v1.CustomResourceDefinition",
        "definitions": convert(definitions),
        "x-source": SOURCE,
    }
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(json.dumps(schema, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(TARGET.relative_to(ROOT))


if __name__ == "__main__":
    main()

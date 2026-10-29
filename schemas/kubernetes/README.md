# Kubernetes CRD object schema

`CustomResourceDefinition.json` is generated from Kubernetes v1.30.0's official
OpenAPI API definition. kubeconform's default schema catalog omits this object;
the local fallback lets CI validate CRDs instead of skipping them.

Regenerate with:

```sh
python scripts/ci/extract-kubernetes-crd-schema.py
```

For an offline regeneration, download the version-pinned URL in the script and
pass its path with `--source`. The generator rewrites OpenAPI component references
to JSON Schema definitions and rejects unknown properties on typed objects.
It preserves explicit `additionalProperties` declarations. The JSON file records
the upstream source URL in `x-source`.

Use the fallback with kubeconform:

```sh
kubeconform -strict -kubernetes-version 1.30.0 \
  -schema-location default \
  -schema-location 'schemas/kubernetes/{{ .ResourceKind }}.json' config/crd/*.yaml
```

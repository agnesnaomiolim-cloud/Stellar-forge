# CI Scripts

This folder contains the checks that the GitHub Actions workflows in [`.github/workflows`](../../.github/workflows) run. Run them from the repository root; most can also be run locally to reproduce a CI failure before pushing. See [`.github/CI_COMMANDS.md`](../../.github/CI_COMMANDS.md) for how CI invokes them.

- [`check-cache-keys.sh`](./check-cache-keys.sh) scans workflow files for raw `actions/cache` usage and checks that `cache-key` / `shared-key` values and cache scopes follow the `<prefix>-<name>` naming convention.
- [`check-stale-todos.sh`](./check-stale-todos.sh) fails if a `TODO` or `FIXME` in `.github/`, `scripts/`, `charts/` or `src/` is not scoped as `TODO(#issue)`, `TODO(@user)` or `TODO(exempt: reason)`.
- [`collect-failure-diagnostics.sh`](./collect-failure-diagnostics.sh) assembles a diagnostics bundle for a failing CI job (run metadata, triage summary, `kubectl` cluster dumps, extra files, sanitized environment) under `/tmp/ci-diagnostics` by default.
- [`extract-crd-json-schemas.py`](./extract-crd-json-schemas.py) generates `schemas/crd/*.json` from the repository's CRD OpenAPI definitions for kubeconform; `--check` fails if the committed schemas have drifted.
- [`helm-upgrade-test.sh`](./helm-upgrade-test.sh) renders `charts/stellar-operator` with production values, re-renders it with upgrade overrides, and asserts that resources, `nodeSelector`, affinity and service ports survive the upgrade.
- [`k8s-compat-smoke-tests.sh`](./k8s-compat-smoke-tests.sh) validates the rendered Helm chart and `config/samples/` with kubeconform against a target Kubernetes version (first argument, default `1.30.0`).
- [`test-db-migrations.sh`](./test-db-migrations.sh) runs the Rust database migration harness (fresh install, existing data, rollback) against the Postgres instance in `DATABASE_URL`, which it requires and never provisions.
- [`validate-config-samples.sh`](./validate-config-samples.sh) checks the YAML in `examples/` and `config/samples/` with kubeconform, skipping CRDs and `_`-prefixed files.
- [`validate-yaml.sh`](./validate-yaml.sh) runs yamllint, the CRD JSON schema drift check and kubeconform against Helm-rendered manifests (repository-wide structure checks live in `scripts/validate-yaml-manifests.py`).

## Running locally

Every script in this folder is invoked from the repository root, the same way CI invokes it. For a local reproduction of a failing CI job, run the matching command from [`.github/CI_COMMANDS.md`](../../.github/CI_COMMANDS.md); the Python script needs Python 3, and `test-db-migrations.sh` expects a reachable Postgres via `DATABASE_URL`.

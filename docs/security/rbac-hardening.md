# Kubernetes RBAC hardening and validator seed custody

This manual defines a restricted **primary reconciliation** profile for the current `stellar-k8s run` binary. Apply [strict-rbac.yaml](../../examples/security/strict-rbac.yaml) with operator namespace `stellar-system`, managed namespace `stellar`, ServiceAccount `stellar-operator`, and arguments `--namespace=stellar-system --watch-namespace=stellar`. Set `POD_NAMESPACE=stellar-system` too. The example namespace is **testnet**; change `stellar.org/network` consistently before using a mainnet workload. Network isolation checks reject mismatches.

This is a source-reviewed reference, not a completed live-cluster security audit. No cluster apply, reconciliation exercise, or audit was executed for this change. The validation gate below is required before production use.

## Boundaries and current limitations

The baseline permits writes only in the two named namespaces, except for patching the named workload Namespace. It grants no Secret writes, RBAC writes, namespace creation, Pod creation, exec/attach, ephemeral-container mutation, or ServiceAccount token creation. Kubernetes RBAC is additive: remove the installation's former broad bindings before evaluating this profile. Inspect bindings first; do not remove unrelated identities' grants.

**`--watch-namespace` does not confine the entire process.** These current source paths use all-namespaces APIs:

| Source | Unavoidable observation / conditional behavior |
| --- | --- |
| `src/preflight.rs` | Lists StellarNodes cluster-wide; lists operator Deployments, ConfigMaps and Leases; gets operator Namespace. |
| `src/controller/peer_discovery.rs` | Lists all StellarNodes, gets their Services, applies `stellar-system/stellar-peers`. The peer namespace is hardcoded by the default config. |
| `src/controller/benchmark/reconciler.rs` | Always watches all StellarBenchmarks and owned Pods. Active benchmarks additionally create Pods, read logs and publish reports/status. |
| `src/controller/snapshot_worker.rs` | Always lists all StellarNodes; nodes with schedules trigger snapshot writes. |
| `src/controller/maintenance/node_drain.rs` | Always lists/watches Nodes; on cordon lists Pods across namespaces and attempts eviction of matching managed pods. |
| `src/controller/quorum/optimizer.rs` | Always lists all StellarNodes; enabled quorum optimization patches nodes and emits events in their own namespaces. |
| `src/controller/topology.rs` | Default `podAntiAffinity: Hard` lists Nodes and sibling StellarNodes cluster-wide. |

The observer ClusterRole supplies these read operations. Node `get` is also used by hardware-generation metrics. Cluster-wide Pod reads reveal Pod configuration, including literal environment variables; Service reads reveal endpoints. The primary Secret watch reads **all Secrets in `stellar`**, including ESO seed Secrets. A watch/list is data access, not only metadata access. `resourceNames` cannot restrict this unfiltered watch. This profile does not make the operator unable to read validator seeds.

The drain worker may attempt operations outside `stellar`; those writes remain denied. Do not enable benchmark execution, quorum optimization, scheduled snapshots, or other extensions in other namespaces and expect permission-error-free operation. Namespace-aware background workers or disable switches are needed for a stronger boundary; RBAC alone cannot provide it.

The stock disk monitor also calls `disk_scaler::get_disk_usage`, which executes `df` using `pods/exec`. In a metrics-enabled build the metrics path invokes this even if automatic disk expansion is disabled. The baseline denies exec, so the probe can log an authorization warning while primary reconciliation continues and disk metrics/expansion are unavailable. There is no supported per-probe disable switch in that path. For healthy disk probes, explicitly apply [disk-probe-rbac.yaml](../../examples/security/disk-probe-rbac.yaml). It grants arbitrary command execution in every Pod in `stellar`; Kubernetes RBAC cannot restrict exec to `df`. A no-exec profile and a clean stock disk-probe audit are incompatible until the implementation changes.

Exec and attach expose both GET/WebSocket and POST API transports. The optional disk extension grants `get,create` for exec to cover those authorization paths. The baseline audit requires **both** verbs denied for exec and attach; checking only POST `create` can miss a GET grant. See the [Pod exec API](https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#connect-get-exec).

## Exact baseline permission map

Server-side apply needs `patch` for existing objects and **also `create` for new objects**, even though both requests use HTTP PATCH. See the official [apply authorization contract](https://kubernetes.io/docs/reference/using-api/server-side-apply/#access-control-and-permissions). Creation cannot be limited with `resourceNames`. The following tables describe every rule in the companion manifest; optional extensions are separate below.

### Role `stellar/stellar-operator`

| API group / resource | Verbs | Source and justification |
| --- | --- | --- |
| `stellar.org/stellarnodes` | `get,list,watch,patch` | `reconciler.rs`: desired-state watch, metadata/finalizer patches; network-isolation sibling checks. No user CR creation/deletion. |
| `stellar.org/stellarnodes/status` | `patch` | Reconcile health, conditions and phases. |
| `stellar.org/stellarnodes/finalizers` | `update` | Owner-reference admission and finalizer authorization; metadata patches use the main resource permission. |
| `apps/deployments,statefulsets` | `get,list,watch,create,patch,delete` | `resources.rs`, `remediation.rs`: owned watches, apply creation, canary updates, restarts, teardown. |
| core `services,persistentvolumeclaims` | `get,list,watch,create,patch,delete` | Owned watches, service apply creation, PVC POST creation/expansion, lifecycle cleanup. |
| core `configmaps` | `get,create,patch,delete` | Node config, alert rules and cleanup, using apply. |
| core `pods` | `get,list,patch,delete` | Health checks, traffic labels and remediation; workload controllers create actual Pods. Observer role additionally supplies list/watch. |
| core `pods/eviction` | `create` | Node-drain worker eviction, only in this namespace; PDB admission still applies. |
| core `secrets` | `get,list,watch` | Primary controller's unfiltered Secret watch, cert/version and seed-related readers; no mutation. |
| `networking.k8s.io/networkpolicies,ingresses` | `create,patch,delete` | Apply isolation/ingress and remove obsolete resources. |
| `policy/poddisruptionbudgets` | `get,list,watch,create,patch,delete` | Owned watch, apply and cleanup of node availability policy. |
| `autoscaling/horizontalpodautoscalers` | `create,patch,delete` | Apply configured HPA; cleanup on node deletion. |
| `autoscaling.k8s.io/verticalpodautoscalers` | `delete` | `delete_vpa` runs even when `vpaConfig` is absent; missing CRD/object 404 is tolerated. |
| core and `events.k8s.io/events` | `create,patch` | Legacy Event publication and kube runtime recorder. |

### Role `stellar-system/stellar-operator-runtime`

| Resource | Verbs / name constraint | Justification |
| --- | --- | --- |
| `apps/deployments` | `list` | Startup preflight. |
| core `configmaps` | `list`; `watch` only on `stellar-operator-config` | Unfiltered preflight list and a feature-flag watch that uses a matching `metadata.name` field selector. |
| core `configmaps` | `get,patch`, name `stellar-peers` | Peer discovery updates and reads the runtime ConfigMap pre-created by the administrator through `strict-rbac.yaml`. If deleted, reapply that manifest; the operator intentionally cannot recreate it. |
| `coordination.k8s.io/leases` | `list,create` | Preflight list and initial leader-lease POST. `create` cannot be restricted with `resourceNames`; it is confined to this namespace. |
| `coordination.k8s.io/leases` | `get,patch`, name `stellar-operator-leader` | Acquire, renew and release lease. |

### ClusterRole `stellar-operator-runtime-observer`

| Resource | Verbs / constraint | Justification |
| --- | --- | --- |
| `stellar.org/stellarnodes` | `list` | Preflight, discovery, snapshots, quorum and topology. |
| `stellar.org/stellarbenchmarks` | `list,watch` | Always-started benchmark controller. |
| core `pods,nodes` | `list,watch` | Benchmark Pod watch and Node-drain watcher; Pod lists on cordon; topology discovery. |
| core `nodes` | `get` | Infrastructure lookup for metrics. |
| core `services` | `get` | Peer-discovery lookup in each discovered node's namespace. |
| core `namespaces` | `get`, names `stellar-system,stellar` | Startup preflight and network-safety checks. |
| core `namespaces` | `patch`, name `stellar` | `pss.rs` reconciles restricted labels using a merge patch; namespace must already exist. RBAC constrains the object, not label keys, so this allows changing any label on `stellar`. |
| `storage.k8s.io/storageclasses` | `get`, names `local-path,local-storage` | Local-storage detection. For automatic expansion add only the actual PVC StorageClass names to this rule; storage-class queries are otherwise denied. |

## Optional feature rules

Add rules only after identifying the enabled feature and namespace. Use a Role/RoleBinding in each reviewed namespace, not a write-capable ClusterRoleBinding just because the background watch is cluster-wide. A shared operator process can still observe feature objects elsewhere and attempt denied writes.

| Feature | Additional API rules | Scope / notes |
| --- | --- | --- |
| Disk probes | core `pods/exec: get,create` | `disk-probe-rbac.yaml`; GET/WebSocket and POST authorization differ by client/server version. Both verbs permit arbitrary commands; see the tradeoff above. |
| ESO seeds | `external-secrets.io/externalsecrets: get,create,patch` | `eso-rbac.yaml`; ESO owns target Secret writes using its own identity. No SecretStore mutation needed by Stellar. |
| VPA enabled | `autoscaling.k8s.io/verticalpodautoscalers: create,patch` | Baseline already permits delete; apply creation requires create. |
| CloudNativePG | `postgresql.cnpg.io/clusters,poolers: create,patch,delete` | `resources.rs` applies/cleans up; CNPG has its own identity. |
| mTLS / cert-manager | core `secrets: get,create,patch`; `cert-manager.io/certificates: create,patch` | Pre-create reviewed operator CA/server Secrets in `stellar-system` and node cert Secrets in `stellar` to omit Secret create. Otherwise create grants cover all Secrets in the namespace. Restrict get/patch to reviewed names in `mtls.rs`; no broad Secret delete needed. |
| Benchmark execution | core `pods: create,get`; `pods/log: get`; `stellar.org/stellarbenchmarks/status: patch`; `stellar.org/stellarbenchmarks/finalizers: update`; `stellar.org/benchmarkreports: create,patch`; core `configmaps: create,patch` | `benchmark/reconciler.rs`, `collector.rs`. Report owner references set `blockOwnerDeletion`, requiring owner finalizer authorization. Grant in benchmark namespace only. Generated benchmark Pods must separately pass PSS. |
| CSI snapshots | `snapshot.storage.k8s.io/volumesnapshots: get,list,create,delete` | `snapshot.rs` / `csi_snapshot.rs`, in each selected node namespace. CSI controller owns snapshot contents. |
| OCI snapshots | `batch/jobs: get,create` | `oci_snapshot.rs` applies push/pull Jobs and polls completion; review registry credentials and any separate cleanup workflow independently. Generated root-running helper containers can fail restricted PSS. |
| Istio | `security.istio.io/peerauthentications,requestauthentications: create,patch,delete`; `networking.istio.io/destinationrules,virtualservices: create,patch,delete` | `service_mesh.rs`; canary routing also applies VirtualServices. |
| Linkerd | `policy.linkerd.io/servers,serverauthorizations: create,patch` | `service_mesh.rs`; service-mesh controllers have separate identities. |
| Spot drain (`NODE_NAME` set) | core `nodes: patch`; core `pods/eviction: create`; node/Pod reads | `spot_drain.rs`; baseline supports in-namespace eviction but not cluster node cordoning. Keep `NODE_NAME` unset in the baseline. |
| REST Kubernetes bearer auth | `authentication.k8s.io/tokenreviews: create`; `authorization.k8s.io/subjectaccessreviews: create` | ClusterRole required because reviews are cluster-scoped. Needed for that auth mode, not controller reconciliation. |
| Forensics | core `pods/ephemeralcontainers: get,patch,update`; relevant logs | Break-glass only; forensic capabilities can be rejected by restricted PSS. |

Blue/green deployment, restore/integrity testing, GitOps decryption, key rotation, cross-cluster workflows and custom plugins are outside this profile. Their permissions depend on configuration and external identities. Review their concrete source calls before enabling them; the table is not a blanket authorization for every operator feature.

## Enforce restricted Pod Security Standards

On a new dedicated testnet namespace, applying `strict-rbac.yaml` sets enforce/audit/warn to `restricted:latest`. The operator's `pss.rs` also forces these labels on reconciliation. A manually pinned version will be overwritten; changing this behavior requires a controller change. Re-evaluate admission whenever Kubernetes is upgraded.

For an existing namespace, first stop reconciliation during migration, audit current Pods and run the enforcement label as a server dry-run:

```bash
kubectl label --overwrite namespace stellar \
  pod-security.kubernetes.io/audit=restricted \
  pod-security.kubernetes.io/warn=restricted
kubectl label --dry-run=server --overwrite namespace stellar \
  pod-security.kubernetes.io/enforce=restricted \
  pod-security.kubernetes.io/enforce-version=latest
# Fix every violation, then apply the reference and restart reconciliation.
kubectl apply -f examples/security/strict-rbac.yaml
```

The main workload builders set non-root UID/GID, RuntimeDefault seccomp, no privilege escalation and drop ALL capabilities. Sidecars, init containers, injected Vault agents, OCI snapshot helpers and optional upgrades need individual admission validation. Admission does not rewrite running Pods. The operator namespace uses audit/warn because the supplied Helm Pod does not assert every restricted field. See the official [namespace label workflow](https://kubernetes.io/docs/tasks/configure-pod-container/enforce-standards-namespace-labels/) and [Restricted requirements](https://kubernetes.io/docs/concepts/security/pod-security-standards/).

Workload Pods need no Kubernetes API RoleBinding to consume a `secretKeyRef`; kubelet retrieves the Secret. The builder currently uses the namespace's default ServiceAccount. Do not bind it to operator RBAC. To prevent token mounting for the ESO-based paths below:

```bash
kubectl patch serviceaccount default -n stellar --type=merge \
  -p '{"automountServiceAccountToken":false}'
```

This namespace-wide default is unsuitable for a Vault Agent path that needs the workload's projected token. Use the ESO path below when avoiding workload API credentials is required.

## Seed interface and version prerequisites

The supported interface is `spec.validatorConfig.seedSecretSource` with exactly one `localRef`, `externalRef`, `csiRef` or `vaultRef`. `kms_secret.rs` implements `externalRef` by applying an ExternalSecret and injecting `<StellarNode-name>-seed` key `STELLAR_CORE_SEED` into the main container's environment. The container image must consume that variable; this is the existing validator seed interface.

The operator currently hardcodes **`external-secrets.io/v1beta1`**. Verify that your installed ESO CRDs serve it before using `externalRef`:

```bash
kubectl get crd externalsecrets.external-secrets.io \
  -o jsonpath='{range .spec.versions[*]}{.name}{" served="}{.served}{"\n"}{end}'
kubectl get crd secretstores.external-secrets.io \
  -o jsonpath='{range .spec.versions[*]}{.name}{" served="}{.served}{"\n"}{end}'
kubectl apply -f examples/security/eso-rbac.yaml
```

Current ESO documentation uses `v1`. A v1-only installation needs an operator API-version compatibility change before `externalRef` works; changing only the example YAML does not fix the controller's requests. Install/maintain ESO separately with its official chart, restrict its namespace scope, and retain its required Secret/token-request permissions. Its ServiceAccount is distinct from Stellar's.

## AWS KMS through Secrets Manager and ESO

This workflow uses a customer-managed symmetric KMS encryption key protecting a Secrets Manager value; it is **not** KMS remote Ed25519 signing. No seed or AWS credentials belong in the StellarNode CR. See [AWS encryption behavior](https://docs.aws.amazon.com/secretsmanager/latest/userguide/security-encryption.html) and [ESO AWS provider](https://external-secrets.io/latest/provider/aws-secrets-manager/).

Prerequisites: EKS with its OIDC provider registered in IAM, ESO able to request a token for `stellar/stellar-seed-reader`, an administrator authenticated to AWS CLI, and a seed JSON file on a protected local path outside Git containing `{"seed":"S..."}`. Set the values for your account:

```bash
export AWS_REGION=us-east-1
export ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
export EKS_CLUSTER=your-cluster
export SEED_JSON=/secure/validator-0.json
export KMS_KEY_ARN="$(aws kms create-key --key-spec SYMMETRIC_DEFAULT \
  --key-usage ENCRYPT_DECRYPT --description 'Stellar seed envelope encryption' \
  --query KeyMetadata.Arn --output text)"
export SECRET_ARN="$(aws secretsmanager create-secret \
  --name stellar/prod/validator-0 --kms-key-id "$KMS_KEY_ARN" \
  --secret-string "file://$SEED_JSON" --query ARN --output text)"
export OIDC_ISSUER="$(aws eks describe-cluster --name "$EKS_CLUSTER" \
  --query cluster.identity.oidc.issuer --output text)"
export OIDC_HOST="${OIDC_ISSUER#https://}"
cat > stellar-seed-trust.json <<EOF
{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{
"Federated":"arn:aws:iam::$ACCOUNT_ID:oidc-provider/$OIDC_HOST"},
"Action":"sts:AssumeRoleWithWebIdentity","Condition":{"StringEquals":{
"$OIDC_HOST:sub":"system:serviceaccount:stellar:stellar-seed-reader",
"$OIDC_HOST:aud":"sts.amazonaws.com"}}}]}
EOF
cat > stellar-seed-policy.json <<EOF
{"Version":"2012-10-17","Statement":[
{"Effect":"Allow","Action":["secretsmanager:GetSecretValue","secretsmanager:DescribeSecret"],"Resource":"$SECRET_ARN"},
{"Effect":"Allow","Action":"kms:Decrypt","Resource":"$KMS_KEY_ARN",
"Condition":{"StringEquals":{"kms:ViaService":"secretsmanager.$AWS_REGION.amazonaws.com",
"kms:EncryptionContext:SecretARN":"$SECRET_ARN"}}}]}
EOF
aws iam create-role --role-name stellar-seed-read \
  --assume-role-policy-document file://stellar-seed-trust.json
aws iam put-role-policy --role-name stellar-seed-read --policy-name validator-0-read \
  --policy-document file://stellar-seed-policy.json
```

The KMS key policy must allow the account to delegate these IAM permissions (the default policy from `create-key` does); an existing custom key policy may need a grant for this reader. The reader cannot encrypt, modify/delete the secret or read other seeds. The provisioning administrator needs separate create/encrypt permissions.

Replace account ARN and region in `externalsecret-aws.yaml`, apply it, then configure an **existing testnet validator** named `validator-0`:

```bash
kubectl apply -f examples/security/externalsecret-aws.yaml
kubectl wait -n stellar --for=condition=Ready secretstore/aws-secrets-manager --timeout=120s
kubectl patch stellarnode validator-0 -n stellar --type=merge -p '{"spec":{"validatorConfig":{"seedSecretSource":{"localRef":null,"csiRef":null,"vaultRef":null,"externalRef":{"name":"validator-0-seed","secretStoreRef":{"name":"aws-secrets-manager","kind":"SecretStore"},"remoteKey":"stellar/prod/validator-0","remoteProperty":"seed","refreshInterval":"1h"}}}}}'
kubectl wait -n stellar --for=condition=Ready externalsecret/validator-0-seed --timeout=120s
```

## HashiCorp Vault through ESO

This executable alternative also uses `externalRef`, avoiding the builder's default workload ServiceAccount and Vault Injector/PSS compatibility assumptions. Prerequisites: Vault on this Kubernetes cluster with TLS and KV-v2 mount `secret/`, Kubernetes auth enabled/configured using Vault's local rotating reviewer token, and Vault's own SA granted `system:auth-delegator`. Configure the Kubernetes auth mount according to [Vault's Kubernetes guide](https://developer.hashicorp.com/vault/docs/auth/kubernetes); do not grant token-review permissions to the validator or Stellar operator. For external Vault, configure a maintained reviewer credential and reachable Kubernetes API instead.

With Vault administrator credentials and a protected seed file containing only the seed:

```bash
export VAULT_ADDR=https://vault.vault.svc:8200
export VAULT_CACERT=/secure/vault-ca.crt
export SEED_FILE=/secure/validator-0.seed
vault kv put secret/stellar/validators/validator-0 seed=@"$SEED_FILE"
vault policy write stellar-validator - <<'EOF'
path "secret/data/stellar/validators/validator-0" {
  capabilities = ["read"]
}
EOF
vault write auth/kubernetes/role/stellar-validator \
  bound_service_account_names=stellar-seed-reader \
  bound_service_account_namespaces=stellar \
  policies=stellar-validator audience=vault ttl=1h
kubectl create configmap vault-ca -n stellar --from-file=ca.crt="$VAULT_CACERT" \
  --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f examples/security/externalsecret-vault.yaml
kubectl wait -n stellar --for=condition=Ready secretstore/vault-secrets --timeout=120s
kubectl patch stellarnode validator-0 -n stellar --type=merge -p '{"spec":{"validatorConfig":{"seedSecretSource":{"localRef":null,"csiRef":null,"vaultRef":null,"externalRef":{"name":"validator-0-seed","secretStoreRef":{"name":"vault-secrets","kind":"SecretStore"},"remoteKey":"stellar/validators/validator-0","remoteProperty":"seed","refreshInterval":"1h"}}}}}'
kubectl wait -n stellar --for=condition=Ready externalsecret/validator-0-seed --timeout=120s
```

The store audience `vault` must match the role and be supported by the deployed Kubernetes/Vault token-review configuration. Keep TLS verification enabled and mount only the CA public certificate in `vault-ca`. ESO needs permission to request the reader SA's token. See the [ESO Vault provider](https://external-secrets.io/v0.20.4/provider/hashicorp-vault/) for that API contract.

`vaultRef` is also implemented: it adds Injector annotations and `STELLAR_SEED_FILE=/vault/secrets/<file>`, with a KV-v2 `.Data.data` template. That path requires a container entrypoint which reads the file, a workload SA/token, an Injector configured for restricted admission, and verified rotation/reload behavior. It is not interchangeable with the ESO example above.

Both ESO workflows materialize plaintext seed bytes in a Kubernetes Secret and in a process environment. Protect the Kubernetes datastore with encryption at rest and restrict readers; KMS-at-rest in AWS does not encrypt the Kubernetes copy. On seed changes, update the remote value (`aws secretsmanager put-secret-value` or `vault kv put`), wait for ExternalSecret Ready, and roll the validator StatefulSet under quorum control. Environment variables do not refresh in existing containers; the current Secret watch's mapper returns no reconcile targets, so do not assume automatic restart. Seed identity changes require the corresponding Stellar validator/quorum procedure. Validate rotation on testnet first; never print Secret data or a full Pod environment in audit logs.

## Apply, validate and record evidence

Use a disposable cluster and a testnet validator fixture with optional features off. Review existing RoleBindings/ClusterRoleBindings for this ServiceAccount, replace broad installation bindings, then apply the manifest as an administrator:

```bash
kubectl get rolebindings -A -o wide
kubectl get clusterrolebindings -o wide
kubectl apply --dry-run=server -f examples/security/strict-rbac.yaml
kubectl apply -f examples/security/strict-rbac.yaml
# Deploy using the named SA and namespace arguments above; install CRDs first.
kubectl get stellarnodes -n stellar -w
kubectl get deploy,statefulset,svc,pvc,networkpolicy,pdb -n stellar
bash examples/security/audit-rbac.sh
```

The audit checks permissions, both PSS admission outcomes using server dry-runs, and current operator logs. Expected RBAC denials must produce `no`; API failures do not count as denial evidence. The negative admission test must report a restricted PodSecurity violation, so a different webhook or permission error does not count as PSS enforcement. A denied disk exec probe is a **failed clean-log gate**, not evidence of a complete security pass. Either retain that known degradation and record it explicitly, or apply the reviewed disk extension and run `EXPECT_DISK_EXEC=yes bash examples/security/audit-rbac.sh`. Neither choice authorizes writes in unrelated namespaces.

For each enabled extension, exercise creation, update, steady-state health, child-resource recreation, and deletion/finalizer cleanup. Exercise leader acquisition and failover. Confirm workloads become Ready and status advances; an empty error search alone proves nothing. Confirm permitted/forbidden operations with impersonation (the auditing administrator needs impersonation rights). Save the rendered manifests, API/ESO/Vault versions, namespace labels, `auth can-i` outputs, PSS dry-run errors, reconcile logs and Ready status as review artifacts **without secret values**.

Reject a review that adds wildcard resources/verbs, cluster-wide workload writes, cross-namespace Secret reads, broad namespace mutation, unexpected Secret writes, operator RBAC on workload SAs, or unreviewed exec privileges. Review extension-specific PSS failures without exempting the production namespace. A live security audit and sign-off remain release requirements; this document does not assert they passed.

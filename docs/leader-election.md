# Leader Election Implementation Guide

## Overview

The Stellar-K8s Operator implements high availability (HA) through leader election, enabling multiple operator replicas to run simultaneously with only one designated leader processing reconciliation at any given time. This ensures:

- **High Availability**: If the leader becomes unavailable, a follower automatically takes over within ~20 seconds
- **Single coordination authority**: Lease updates use Kubernetes resource versions so competing claims cannot overwrite each other
- **Zero Downtime**: Leader transitions happen without interrupting cluster operations
- **Automatic Failover**: No manual intervention required for replica failures

> **Multi-cluster requirement:** A Kubernetes Lease is scoped to one Kubernetes API server. Separate DR clusters must all use the same coordination-cluster kubeconfig and Lease namespace or each cluster can elect its own leader. The Helm chart rejects `leaderElection.global: true` unless both are configured.

The Lease prevents two pods from being the current recorded holder. A process suspended beyond the TTL can still have an already-issued external request complete after a new holder takes over; sinks requiring hard fencing must also validate a fencing token. Do not treat a Kubernetes Lease alone as a transactional fence for in-flight S3 or Kafka requests.

## Architecture

### Leader Election Mechanism

The operator uses Kubernetes `coordination.k8s.io/v1` Lease resources for leader election:

1. **Lease Creation**: Each operator replica attempts to create/renew a Lease named `stellar-operator-leader`
2. **Lease Holder**: The replica that successfully holds the Lease is the leader
3. **Renewal**: The leader continuously renews its lease every 5 seconds
4. **Expiry**: If a leader crashes or stops renewing, its lease expires after 15 seconds
5. **Takeover**: Any operator using the shared coordination API retries every second and claims the expired Lease using an optimistic resource-version update

Kubernetes calls have a three-second timeout. A readiness check failure causes the leader to stop renewing and release its Lease. If release cannot reach the coordination API, failover falls back to TTL expiry.

### Global Coordination Cluster

Create a kubeconfig Secret in each operator's local namespace that grants narrowly scoped `get`, `create`, and `update` access to the named Lease in one shared coordination cluster. Mount the same credentials and use the same Lease namespace in every DR cluster:

```yaml
leaderElection:
  global: true
  kubeconfigSecretName: stellar-coordination-kubeconfig
  kubeconfigSecretKey: config
  leaseName: stellar-operator-leader
  leaseNamespace: stellar-coordination
```

The Secret's `config` key contains a kubeconfig for the shared coordinator. Credentials must be valid from all DR clusters and should be stored/rotated through the deployment's secret manager. The operator uses its local in-cluster client for resources in that cluster and the configured kubeconfig only for election.

The pod UID is used as the holder identity, preventing same-named operator pods in different clusters from appearing to be the same holder.

### Key Components

```rust
// In src/controller/leader.rs

const LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;      // Lease validity period
const RENEW_INTERVAL: Duration = Duration::from_secs(5);   // Renewal frequency
const RETRY_INTERVAL: Duration = Duration::from_secs(1);   // Retry on failure
const API_TIMEOUT: Duration = Duration::from_secs(3);      // Per-request timeout

let election = LeaderElectionHandle::start(shared_coordination_client, None, None, None)?;
```

### Replica Identity

Each operator replica is identified by:

- **POD_UID**: Globally unique pod identity for shared multi-cluster election
- **OPERATOR_LEASE_NAMESPACE**: Namespace containing the Lease on the coordination cluster

The deployment manifest injects `POD_UID`. The Helm chart configures the Lease namespace and mounts the shared kubeconfig when `leaderElection.global` is enabled.

## Deployment Configuration

### Single Replica (Default)

```yaml
replicaCount: 1
```

Only one operator instance runs. Leader election still works but is unused.

### High Availability Setup (Recommended)

```yaml
replicaCount: 3

podDisruptionBudget:
    enabled: true
    minAvailable: 1
```

Deploy 3 operator replicas with:

- Pod anti-affinity spreading replicas across different nodes
- Pod Disruption Budget ensuring ≥1 replica stays available during maintenance
- Automatic leader election coordinating which replica processes reconciliation

## Installation

### Install with Default (Single Replica)

```bash
helm install stellar-operator ./charts/stellar-operator
```

### Install with High Availability (3 Replicas)

```bash
helm install stellar-operator ./charts/stellar-operator \
  --set replicaCount=3 \
  --set podDisruptionBudget.enabled=true
```

Or use the provided HA values file:

```bash
helm install stellar-operator ./charts/stellar-operator \
  -f ./charts/stellar-operator/values-ha.yaml
```

### Upgrade Existing Single-Replica Deployment to HA

```bash
helm upgrade stellar-operator ./charts/stellar-operator \
  --set replicaCount=3 \
  --set podDisruptionBudget.enabled=true
```

## Monitoring Leader Election

### Check Current Leader

```bash
# View the Lease resource
kubectl get lease -n stellar-operator stellar-operator-leader -o yaml

# Shows:
# spec:
#   holderIdentity: stellar-operator-pod-1
#   acquireTime: 2024-05-31T10:15:23Z
#   renewTime: 2024-05-31T10:15:33Z
#   leaseDurationSeconds: 15
```

### View Operator Status

```bash
# Check if an operator pod is the leader
kubectl logs -n stellar-operator stellar-operator-pod-1 | grep "Acquired leadership"

# Non-leader output:
# kubectl logs -n stellar-operator stellar-operator-pod-2
# [2024-05-31T10:15:25Z] Lost leadership (will retry)
```

### Metrics

With metrics enabled, the operator exposes:

```
# HELP stellar_operator_leader_status Whether the operator is the current leader
# TYPE stellar_operator_leader_status gauge
stellar_operator_leader_status{instance="stellar-operator-pod-1"} 1
stellar_operator_leader_status{instance="stellar-operator-pod-2"} 0
stellar_operator_leader_status{instance="stellar-operator-pod-3"} 0

# HELP stellar_operator_uptime_seconds Operator uptime in seconds
# TYPE stellar_operator_uptime_seconds counter
stellar_operator_uptime_seconds{instance="stellar-operator-pod-1"} 3600
```

## Failover Scenarios

### Scenario 1: Leader Pod Crashes

Timeline:

- **T=0s**: Leader crashes, Pod enters CrashLoopBackOff
- **T=5s**: Followers notice no lease renewal
- **T=15s**: Lease expires
- **T=16s**: A follower acquires the lease and becomes leader
- **T=20s**: New leader starts processing reconciliation

### Scenario 2: Leader Pod Becomes Unresponsive (Network Partition)

Timeline:

- **T=0s**: Network partition isolates leader
- **T=5s**: Leader's next renewal fails and it steps down locally
- **T=15s**: Lease expires
- **T≈16s**: A responding operator acquires the expired Lease
- **T≈19s**: New leader resumes reconciliation after the bounded API request

If the leader's readiness probe fails while the process remains alive, it stops renewing and attempts to release the Lease. If the process is killed before release, failover uses the TTL.

### Scenario 3: Planned Maintenance (Node Drain)

Timeline:

- **T=0s**: Cluster admin runs `kubectl drain node-1` (where leader runs)
- **T=10s**: Kubernetes eviction grace period begins
- **T=30s**: Pod is forcefully terminated
- **T=31s**: A follower on another node acquires the lease
- **T=35s**: New leader starts processing reconciliation

With Pod Disruption Budget enabled:

- Drain process respects `minAvailable: 1` constraint
- At least one operator pod stays available

## Behavior of Different Replica Types

### Leader Pod

**Responsibilities**:

- Processes reconciliation for StellarNode and other CRDs
- Updates status subresources
- Creates/updates Kubernetes resources (Deployments, Services, etc.)
- Logs reconciliation events

**Monitoring**:

- Serves metrics on `:9090/metrics`
- Serves liveness and readiness probes when the REST API is enabled
- Logs show `Acquired leadership: stellar-operator-leader`

### Follower Pod

**Responsibilities**:

- Waits for the shared Lease before starting the operator
- Takes over when the Lease is released or expires
- Does not reconcile or publish SCP telemetry while it does not hold the Lease

Standby pods remain running while waiting for the Lease. In the current CLI lifecycle, they do not start the operator REST API until elected.

## Environment Variables

The following environment variables control leader election:

| Variable | Default | Purpose |
| --- | --- | --- |
| `LEADER_ELECTION_KUBECONFIG` | Unset (local API client) | Kubeconfig for the shared coordination cluster |
| `OPERATOR_LEASE_NAMESPACE` | `POD_NAMESPACE` or `default` | Lease namespace on the coordination cluster |
| `OPERATOR_LEASE_NAME` | `stellar-operator-leader` | Lease resource name |
| `POD_UID` | Process-specific fallback | Globally unique Lease holder identity |
| `REQUIRE_SHARED_LEADER_ELECTION` | `false` | Fail startup if the coordination kubeconfig is missing |

These are automatically set by the Kubernetes downward API in the deployment manifest.

## RBAC Requirements

For single-cluster use, the operator ServiceAccount needs permissions for Leases. In global mode, grant the identity in the shared kubeconfig the same scoped permissions on the coordination cluster:

```yaml
- apiGroups: ["coordination.k8s.io"]
  resources: ["leases"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

The local-cluster role is pre-configured in the Helm chart's RBAC template. The chart cannot create RBAC in the separate coordination cluster.

## Testing

### Unit Tests

Run the leader election test suite:

```bash
cargo test --test leader_election_test
```

Tests cover:

- Single leader election invariant
- Concurrent leadership transitions
- Lease renewal mechanics
- Failover scenarios
- Scaling with multiple replicas

### Integration Test

Deploy to a Kubernetes cluster and verify:

```bash
# Deploy 3 replicas
helm upgrade stellar-operator ./charts/stellar-operator \
  --set replicaCount=3 \
  --set leaderElection.global=true \
  --set leaderElection.kubeconfigSecretName=stellar-coordination-kubeconfig \
  --set leaderElection.leaseNamespace=stellar-coordination

# Wait for deployment to settle
kubectl rollout status deployment/stellar-operator -n stellar-operator

# Read the shared Lease (holderIdentity is the Kubernetes pod UID)
LEADER_UID=$(kubectl --kubeconfig ./coordination.kubeconfig \
  get lease -n stellar-coordination stellar-operator-leader \
  -o jsonpath='{.spec.holderIdentity}')
echo "Current leader pod UID: $LEADER_UID"

# Resolve the UID to a pod in the cluster currently holding leadership, then
# terminate that pod to exercise TTL-based takeover.
LEADER_POD=$(kubectl get pods -n stellar-operator -o json \
  | jq -r --arg uid "$LEADER_UID" '.items[] | select(.metadata.uid == $uid) | .metadata.name')
kubectl delete pod "$LEADER_POD" -n stellar-operator

# Watch the shared coordination Lease
watch kubectl --kubeconfig ./coordination.kubeconfig \
  get lease -n stellar-coordination stellar-operator-leader -o yaml

# Verify reconciliation continues
kubectl logs -f deployment/stellar-operator -n stellar-operator | grep "reconciling"
```

## Troubleshooting

### Problem: Multiple Leaders Detected

**Symptom**: Lease shows multiple pods with leader status
**Cause**: Usually indicates a serious clock skew or API server issue
**Solution**:

1. Check NTP sync on all nodes: `timedatectl status`
2. Verify API server health: `kubectl get componentstatuses`
3. Check operator logs for API errors

### Problem: No Leader Election Activity

**Symptom**: Lease is never created or updated
**Cause**: Missing RBAC permissions
**Solution**: Verify operator ServiceAccount has Lease permissions

```bash
kubectl auth can-i patch leases --as=system:serviceaccount:stellar-operator:stellar-operator
```

### Problem: Leadership Flapping

**Symptom**: Leader changes every few seconds
**Cause**: Usually high latency or network issues
**Solution**:

1. Check network connectivity between API server and operator pods
2. Review operator logs for timeout messages
3. Increase `LEASE_DURATION_SECS` in code if needed (currently 15s)

### Problem: Long Failover Time

**Symptom**: Leader failure takes >30 seconds to trigger failover
**Cause**: Follower pods may be slow to renew attempts
**Solution**: Check Pod resource limits and node capacity

## Performance Considerations

### Overhead

- **Leader**: Minimal overhead from lease renewal (5s interval)
- **Followers**: Minimal overhead from lease acquisition attempts (1s interval)
- **Lease creation**: ~10-50ms per operation

### Scalability

- Tested with 3-10 replicas
- No significant performance degradation with more replicas
- Lease coordination is O(1) regardless of replica count

### Lease Timing

Current configuration:

- **Lease Duration**: 15 seconds
- **Renewal Interval**: 5 seconds
- **Retry Interval**: 1 second
- **Per-request timeout**: 3 seconds

This means:

- Crash failover starts after the 15-second TTL and completes after the retry/API request, usually within the configured TTL plus API latency.

## Migration Guide

### From Single Replica to HA

1. **Phase 1: Deploy Additional Replicas** (no downtime)

    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=3
    ```

    - New replicas start as followers
    - Existing leader continues processing
    - Old leader automatically becomes a follower

2. **Phase 2: Verify** (5 minutes)
    - Check all pods are Running
    - Verify only one leader in Lease resource
    - Monitor logs for reconciliation activity

3. **Phase 3: Enable PDB** (optional)
    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=3 \
      --set podDisruptionBudget.enabled=true
    ```

### From HA Back to Single Replica

1. Scale down to 1 replica:

    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=1
    ```

    - Kubernetes will terminate follower pods
    - If leader is terminated, the remaining pod becomes leader
    - No data loss (state is in etcd)

## Advanced Configuration

### Custom Lease Duration

To adjust lease timing, edit `src/controller/leader.rs`:

```rust
const LEASE_DURATION_SECS: i32 = 30;    // Increase for higher latency networks
const RENEW_INTERVAL: Duration = Duration::from_secs(10);
const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const API_TIMEOUT: Duration = Duration::from_secs(3);
```

Then rebuild the operator image.

### Custom Leader Identity

The operator uses `POD_UID`, which is injected by the chart and unique across clusters. If running outside Kubernetes, set `POD_UID` or `POD_NAME` explicitly to a unique value:

```yaml
env:
    - name: POD_UID
      value: "custom-operator-identity"
```

## Related Resources

- [Kubernetes Lease API Documentation](https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/lease-v1/)
- [Operator Pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
- [Leader Election Implementations](https://github.com/kubernetes-sigs/controller-runtime/tree/main/pkg/leaderelection)

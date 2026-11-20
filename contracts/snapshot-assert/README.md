# Snapshot Assert Contract

A Soroban smart contract for validating, synchronizing, and asserting state root consistency across distributed Kubernetes clusters running Stellar infrastructure. It allows registered clusters to submit state root assertions at specific ledger indices and automatically detects state divergences between clusters.

## Features

- **Cluster Registry**: Dynamic registration of authorized cluster identifiers (`register_cluster`) tracked in persistent contract storage.
- **State Root Submissions**: Clusters submit state root commitments for target ledger sequences (`submit_snapshot`) with automatic ledger timestamp recording.
- **Automated Divergence Detection**: Automatically compares candidate state roots against reference cluster roots (`compare_snapshot`) and determines whether snapshots are `Valid` or `Diverged`.
- **On-chain Divergence Events**: Publishes `StateDivergenceDetected` contract events containing the ledger index, reference cluster, reference root, and all conflicting cluster identifiers whenever a divergence is identified.
- **Stale Entry Pruning**: Automatically purges historical snapshot records older than `MAX_ROOT_ENTRIES` (1,000 ledgers) upon new submissions to prevent unbounded storage growth.

## Architecture & Workflow

```text
Cluster A ──submit_snapshot──▶ [Persistent Storage] ◀──submit_snapshot── Cluster B
                                      │
                              compare_snapshot()
                                      │
                     ┌────────────────┴────────────────┐
                     ▼                                 ▼
           Roots Match: Valid              Roots Differ: Diverged
                                                       │
                                            StateDivergenceDetected
                                                 (Event Emitted)
```

1. **Cluster Registration**: A cluster calls `register_cluster(cluster_id)` before submitting snapshots.
2. **Snapshot Submission**: Authorized clusters invoke `submit_snapshot(cluster_id, ledger_index, state_root)` on newly finalized ledger states.
3. **Consensus & Assertion**: The contract stores the snapshot, checks for divergence across all registered clusters for that ledger, and returns a `SnapshotAssertion` result.
4. **Dispute / Divergence Reporting**: If any cluster reports a mismatched state root, `compare_roots` flags `has_divergence = true` and emits an event for off-chain monitoring.

## Public Functions

All public entry points are implemented in [`src/lib.rs`](src/lib.rs):

| Function | Signature | Description |
|---|---|---|
| `__constructor` | `__constructor(env: Env)` | Initializes contract instance storage with `init = true`. |
| `register_cluster` | `register_cluster(env: Env, cluster_id: String) -> Result<(), SnapshotError>` | Registers a new cluster identifier in the contract's persistent cluster registry. Rejects empty strings with `SnapshotError::InvalidCluster`. |
| `submit_snapshot` | `submit_snapshot(env: Env, cluster_id: String, ledger_index: u32, state_root: String) -> Result<SnapshotAssertion, SnapshotError>` | Submits a state root snapshot for a cluster at `ledger_index`, purges stale entries past 1,000 ledgers, verifies cluster registration, checks for state divergences, and returns a `SnapshotAssertion`. |
| `get_snapshot_for_cluster` | `get_snapshot_for_cluster(env: Env, ledger_index: u32, cluster_id: String) -> Option<StateRootEntry>` | Read-only query to fetch the recorded `StateRootEntry` (ledger index, cluster ID, state root, and submission timestamp) for a specific cluster and ledger index. |
| `compare_snapshot` | `compare_snapshot(env: Env, ledger_index: u32) -> Result<SnapshotComparison, SnapshotError>` | Compares all registered clusters' submitted roots for `ledger_index` against the reference cluster root. Publishes `StateDivergenceDetected` event if a divergence exists. |

## Data Types & Errors

### Types

- `SnapshotStatus`: `Valid` | `Diverged`
- `StateRootEntry`: `{ ledger_index: u32, cluster_id: String, state_root: String, submitted_at: u64 }`
- `SnapshotAssertion`: `{ ledger_index: u32, cluster_id: String, state_root: String, status: SnapshotStatus }`
- `SnapshotComparison`: `{ ledger_index: u32, reference_cluster: String, reference_root: String, conflicting_clusters: Vec<String>, has_divergence: bool }`
- `SnapshotDiff`: `{ ledger_index: u32, cluster_id: String, expected_root: String, actual_root: String }`

### Errors (`SnapshotError`)

- `InvalidCluster` (`1`): Provided cluster ID is empty or not registered.
- `MissingStateRoot` (`2`): No state roots found for the specified ledger index.
- `InvalidLedgerIndex` (`3`): Ledger index is 0 or invalid.
- `DivergenceDetected` (`4`): A state mismatch was identified between clusters.
- `StorageLimitExceeded` (`5`): Storage budget or capacity threshold reached.

## Building

To compile the contract to WebAssembly:

```bash
cd contracts/snapshot-assert
cargo build --target wasm32-unknown-unknown --release
```

## Testing

To run the unit and integration tests:

```bash
cd contracts/snapshot-assert
cargo test
```

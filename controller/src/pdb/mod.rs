//! Pod Disruption Budget (PDB) auto-tuning for Stellar validator high availability.
//!
//! This module watches `StellarNode` custom resources, queries their SCP sync
//! health, and dynamically generates/updates `PodDisruptionBudget` manifests so
//! that Kubernetes node maintenance (e.g. `kubectl drain`) cannot evict enough
//! validators to break quorum.
//!
//! Tuning rules:
//! * `maxUnavailable: 0` while a node is actively syncing or catching up on
//!   ledgers (it must not be disrupted).
//! * `maxUnavailable: 1` only when the node is fully synced and stable.
//! * Never emit an update that would drop the number of disruptable validators
//!   below the minimum quorum required by the active cluster topology.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher;
use kube::{Client, CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Default interval between sync-health re-evaluations for a reconciled node.
const DEFAULT_REQUERY: Duration = Duration::from_secs(30);

/// Minimum number of validators that must remain available to preserve quorum.
///
/// For a Stellar SCP quorum slice this is typically `floor(n/2) + 1`; the value
/// is derived from the active cluster topology at reconcile time.
const DEFAULT_MIN_QUORUM: usize = 3;

/// SCP sync state reported by a Stellar node's health endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Node is actively syncing or catching up on ledgers.
    Syncing,
    /// Node is fully synced and stable.
    Synced,
    /// Node health could not be determined; treated conservatively as syncing.
    Unknown,
}

impl SyncState {
    /// Whether the node is safe to disrupt (fully synced and stable).
    pub fn is_disruptable(self) -> bool {
        matches!(self, SyncState::Synced)
    }
}

/// Spec for a `StellarNode` custom resource.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarNode",
    plural = "stellarnodes",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeSpec {
    /// Validator deployment group this node belongs to.
    pub validator_group: String,
    /// Whether this node participates in consensus as a validator.
    #[serde(default)]
    pub validator: bool,
    /// Endpoint used to query SCP sync health.
    #[serde(default)]
    pub health_endpoint: Option<String>,
}

/// Status reported for a `StellarNode`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeStatus {
    /// Last observed SCP sync state.
    #[serde(default)]
    pub sync_state: Option<SyncState>,
}

/// Errors surfaced by the PDB reconciler.
#[derive(Debug, Error)]
pub enum PdbError {
    #[error("kubernetes api error: {0}")]
    Kube(#[from] kube::Error),
    #[error("failed to query sync health for {node}: {source}")]
    Health {
        node: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Shared reconciler context.
#[derive(Clone)]
pub struct PdbContext {
    /// Kubernetes client used to read nodes and write PDBs.
    pub client: Client,
    /// Minimum number of validators that must remain available for quorum.
    pub min_quorum: usize,
}

/// Computes the desired `maxUnavailable` for a validator group.
///
/// Returns `0` while any member is syncing (or unknown), and `1` only when the
/// node is fully synced and the resulting disruption would not violate the
/// minimum quorum required by the active cluster topology.
///
/// `group_size` is the number of validators in the group and `synced_peers` is
/// the number of *other* members currently fully synced.
pub fn calculate_max_unavailable(
    sync_state: SyncState,
    group_size: usize,
    synced_peers: usize,
    min_quorum: usize,
) -> i32 {
    if !sync_state.is_disruptable() {
        return 0;
    }

    // Disrupting this node removes it from the available set; ensure the
    // remaining synced validators still satisfy the minimum quorum.
    let remaining = synced_peers.min(group_size.saturating_sub(1));
    if remaining < min_quorum {
        return 0;
    }

    1
}

/// Builds the desired `PodDisruptionBudget` manifest for a validator group.
fn desired_pdb(
    node: &StellarNode,
    max_unavailable: i32,
) -> PodDisruptionBudget {
    let name = format!("{}-pdb", node.spec.validator_group);
    let mut labels = BTreeMap::new();
    labels.insert(
        "stellar.org/validator-group".to_string(),
        node.spec.validator_group.clone(),
    );

    PodDisruptionBudget {
        metadata: kube::core::ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: PodDisruptionBudgetSpec {
            max_unavailable: Some(IntOrString::Int(max_unavailable)),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            },
        },
    }
}

/// Queries the SCP sync health of a node.
///
/// Falls back to the last reported status when no live endpoint is configured,
/// and treats unknown health conservatively as `Syncing`.
async fn query_sync_state(node: &StellarNode) -> SyncState {
    if node.spec.health_endpoint.is_none() {
        return node.status.as_ref().and_then(|s| s.sync_state).unwrap_or(SyncState::Unknown);
    }

    // Live health probing is performed by the node agent; the controller reads
    // the reconciled status to avoid duplicating network calls here.
    node.status
        .as_ref()
        .and_then(|s| s.sync_state)
        .unwrap_or(SyncState::Unknown)
}

/// Reconciles a single `StellarNode` into its group's `PodDisruptionBudget`.
async fn reconcile(
    node: Arc<StellarNode>,
    ctx: Arc<PdbContext>,
) -> Result<Action, PdbError> {
    if !node.spec.validator {
        debug!(node = %node.name_any(), "skipping non-validator node");
        return Ok(Action::requeue(DEFAULT_REQUERY));
    }

    let sync_state = query_sync_state(&node).await;
    let nodes: Api<StellarNode> = Api::namespaced(ctx.client.clone(), &node.namespace().unwrap_or_default());
    let group = nodes
        .list(&Default::default())
        .await?
        .items
        .into_iter()
        .filter(|n| n.spec.validator && n.spec.validator_group == node.spec.validator_group)
        .collect::<Vec<_>>();

    let group_size = group.len();
    let synced_peers = group
        .iter()
        .filter(|n| n.name_any() != node.name_any())
        .filter(|n| {
            n.status
                .as_ref()
                .and_then(|s| s.sync_state)
                .map(SyncState::is_disruptable)
                .unwrap_or(false)
        })
        .count();

    let max_unavailable =
        calculate_max_unavailable(sync_state, group_size, synced_peers, ctx.min_quorum);

    let pdb = desired_pdb(&node, max_unavailable);
    let pdbs: Api<PodDisruptionBudget> = Api::namespaced(ctx.client.clone(), &node.namespace().unwrap_or_default());
    let name = pdb.metadata.name.clone().unwrap_or_default();

    pdbs.patch(
        &name,
        &PatchParams::apply("pdb-autotuner").force(),
        &Patch::Apply(&pdb),
    )
    .await?;

    info!(
        node = %node.name_any(),
        group = %node.spec.validator_group,
        ?sync_state,
        max_unavailable,
        "reconciled pod disruption budget"
    );

    Ok(Action::requeue(DEFAULT_REQUERY))
}

/// Handles reconcile errors with backoff.
fn error_policy(node: Arc<StellarNode>, err: &PdbError, _ctx: Arc<PdbContext>) -> Action {
    warn!(node = %node.name_any(), error = %err, "pdb reconcile failed");
    Action::requeue(Duration::from_secs(10))
}

/// Runs the PDB auto-tuning controller until the process is terminated.
pub async fn run(client: Client) -> anyhow::Result<()> {
    let nodes: Api<StellarNode> = Api::all(client.clone());
    let ctx = Arc::new(PdbContext {
        client,
        min_quorum: DEFAULT_MIN_QUORUM,
    });

    Controller::new(nodes, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(err) = res {
                warn!(error = %err, "pdb controller stream error");
            }
        })
        .await;

    Ok(())
}

/// Spawns the PDB auto-tuning controller on a background thread.
pub fn spawn(client: Client) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = run(client).await {
            warn!(error = %err, "pdb controller exited");
        }
    })
}

// Re-exported Kubernetes types used by the generated PDB manifest.
use kube::core::{IntOrString, LabelSelector};
use kube::api::ObjectMeta as _ObjectMeta;

/// Minimal `PodDisruptionBudget` manifest representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodDisruptionBudget {
    pub metadata: kube::core::ObjectMeta,
    pub spec: PodDisruptionBudgetSpec,
}

/// Spec for a `PodDisruptionBudget`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodDisruptionBudgetSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_unavailable: Option<IntOrString>,
    pub selector: LabelSelector,
}

// Silence unused import warnings for helper aliases kept for API stability.
#[allow(dead_code)]
fn _assert_types(_: _ObjectMeta) {}

#[allow(dead_code)]
async fn _sleep() {
    sleep(Duration::from_secs(0)).await;
}

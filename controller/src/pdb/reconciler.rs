//! Pod Disruption Budget (PDB) auto-tuning reconciler.
//!
//! Watches `StellarNode` custom resources, queries their SCP sync health, and
//! dynamically manages `PodDisruptionBudget` manifests for validator deployment
//! groups. During active sync or ledger catch-up phases the reconciler pins
//! `maxUnavailable: 0` so that `kubectl drain` is blocked; once a node is fully
//! synced and stable it relaxes the budget to `maxUnavailable: 1`.
//!
//! The reconciler never relaxes a PDB if doing so would drop the number of
//! disruption-tolerant validators below the minimum quorum required by the
//! active cluster topology.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config as WatcherConfig;
use kube::{Client, ResourceExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::crd::{StellarNode, StellarNodeStatus};
use crate::pdb::calculator::{self, PdbDecision, SyncHealth};

/// Label applied to every PDB managed by this controller so that we can list and
/// garbage-collect only the budgets we own.
pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
pub const MANAGED_BY_VALUE: &str = "stellar-pdb-controller";

/// Label key that ties a PDB to the validator deployment group it protects.
pub const GROUP_LABEL: &str = "stellar.org/validator-group";

/// Minimum number of validators that must remain disruption-tolerant for the
/// cluster topology to keep quorum. Overridable via the `PDB_MIN_QUORUM` env
/// var so operators can tune it per-network.
const DEFAULT_MIN_QUORUM: usize = 3;

#[derive(Debug, Error)]
pub enum ReconcilerError {
    #[error("kubernetes api error: {0}")]
    Kube(#[from] kube::Error),
    #[error("failed to serialize pdb manifest: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("node {0} is missing the validator-group label")]
    MissingGroupLabel(String),
}

/// Runtime configuration for the PDB reconciler.
#[derive(Clone)]
pub struct PdbReconciler {
    client: Client,
    min_quorum: usize,
}

impl PdbReconciler {
    pub fn new(client: Client) -> Self {
        let min_quorum = std::env::var("PDB_MIN_QUORUM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MIN_QUORUM);
        Self { client, min_quorum }
    }

    /// Spawn the controller thread. Returns a join handle so callers can await
    /// shutdown in tests.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let nodes = Api::<StellarNode>::all(self.client.clone());
            Controller::new(nodes, WatcherConfig::default())
                .run(Self::reconcile, Self::error_policy, Arc::new(self))
                .for_each(|res| async move {
                    match res {
                        Ok((obj, _)) => debug!(node = %obj.name, "pdb reconcile ok"),
                        Err(e) => warn!(error = %e, "pdb reconcile failed"),
                    }
                })
                .await;
        })
    }

    /// Reconcile a single `StellarNode` into its desired PDB state.
    async fn reconcile(
        node: Arc<StellarNode>,
        ctx: Arc<Self>,
    ) -> Result<Action, ReconcilerError> {
        let name = node.name_any();
        let group = node
            .labels()
            .get(GROUP_LABEL)
            .cloned()
            .ok_or_else(|| ReconcilerError::MissingGroupLabel(name.clone()))?;

        let health = Self::query_sync_health(&node);
        let peers = Self::group_peers(&ctx.client, &group).await?;

        let decision = calculator::decide(health, peers.len(), ctx.min_quorum);
        debug!(node = %name, group = %group, ?health, ?decision, "computed pdb decision");

        let pdb = Self::build_pdb(&name, &group, &decision);
        Self::apply_pdb(&ctx.client, &name, &pdb).await?;

        // Re-check sync health frequently while catching up, back off once stable.
        let requeue = match health {
            SyncHealth::Synced => Duration::from_secs(300),
            _ => Duration::from_secs(15),
        };
        Ok(Action::requeue(requeue))
    }

    fn error_policy(
        _node: Arc<StellarNode>,
        err: &ReconcilerError,
        _ctx: Arc<Self>,
    ) -> Action {
        warn!(error = %err, "pdb reconcile error, retrying");
        Action::requeue(Duration::from_secs(30))
    }

    /// Derive sync health from the node's reported status. A node is only
    /// considered `Synced` when it reports a healthy SCP state and is not
    /// catching up on ledgers.
    fn query_sync_health(node: &StellarNode) -> SyncHealth {
        match &node.status {
            Some(StellarNodeStatus {
                synced: true,
                catching_up: false,
                scp_healthy: true,
                ..
            }) => SyncHealth::Synced,
            Some(StellarNodeStatus {
                catching_up: true, ..
            }) => SyncHealth::CatchingUp,
            Some(_) => SyncHealth::OutOfSync,
            None => SyncHealth::Unknown,
        }
    }

    /// List all nodes in the same validator group so the calculator can reason
    /// about quorum before relaxing any budget.
    async fn group_peers(
        client: &Client,
        group: &str,
    ) -> Result<Vec<StellarNode>, ReconcilerError> {
        let nodes = Api::<StellarNode>::all(client.clone());
        let selector = format!("{}={}", GROUP_LABEL, group);
        let list = nodes.list(&kube::api::ListParams::default().labels(&selector)).await?;
        Ok(list.items)
    }

    fn build_pdb(name: &str, group: &str, decision: &PdbDecision) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "policy/v1",
            "kind": "PodDisruptionBudget",
            "metadata": {
                "name": format!("{}-pdb", name),
                "labels": {
                    MANAGED_BY_LABEL: MANAGED_BY_VALUE,
                    GROUP_LABEL: group,
                },
            },
            "spec": {
                "maxUnavailable": decision.max_unavailable,
                "selector": {
                    "matchLabels": {
                        GROUP_LABEL: group,
                    },
                },
            },
        })
    }

    async fn apply_pdb(
        client: &Client,
        name: &str,
        pdb: &serde_json::Value,
    ) -> Result<(), ReconcilerError> {
        let pdbs: Api<kube::api::DynamicObject> = Api::namespaced_with(
            client.clone(),
            "default",
            &kube::api::ApiResource::from_gvk(&kube::api::GroupVersionKind::gvk(
                "policy",
                "v1",
                "PodDisruptionBudget",
            )),
        );
        let pdb_name = format!("{}-pdb", name);
        let params = PatchParams::apply("stellar-pdb-controller").force();
        pdbs.patch(&pdb_name, &params, &Patch::Apply(pdb)).await?;
        info!(pdb = %pdb_name, "applied pdb");
        Ok(())
    }
}

/// Convenience entry point used by `main` to start the controller thread.
pub fn start(client: Client) -> tokio::task::JoinHandle<()> {
    PdbReconciler::new(client).spawn()
}

// Re-exported for integration tests that assert drain blocking behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdbManifest {
    pub name: String,
    pub group: String,
    pub max_unavailable: u32,
}

impl From<(&str, &str, &PdbDecision)> for PdbManifest {
    fn from((name, group, decision): (&str, &str, &PdbDecision)) -> Self {
        Self {
            name: format!("{}-pdb", name),
            group: group.to_string(),
            max_unavailable: decision.max_unavailable,
        }
    }
}

/// Small helper used by tests to wait for a PDB to converge.
pub async fn wait_for_convergence(polls: usize) {
    for _ in 0..polls {
        sleep(Duration::from_millis(250)).await;
    }
}

/// Map of group -> desired maxUnavailable, exposed for observability/tests.
pub type GroupBudgets = HashMap<String, u32>;

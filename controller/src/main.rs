use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher;
use kube::{Client, CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{error, info, warn};

/// StellarNode is the custom resource managed by this controller. It represents a
/// single Stellar validator node deployment group.
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1",
    kind = "StellarNode",
    plural = "stellarnodes",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeSpec {
    /// Name of the validator deployment group this node belongs to.
    pub deployment_group: String,
    /// Minimum number of nodes in the group that must remain available to
    /// preserve quorum for the active cluster topology.
    pub min_quorum: i32,
    /// Total number of replicas in the deployment group.
    pub replicas: i32,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeStatus {
    /// Whether the node is fully synced and stable.
    #[serde(default)]
    pub synced: bool,
    /// Whether the node is actively syncing or catching up on ledgers.
    #[serde(default)]
    pub syncing: bool,
    /// Latest ledger sequence observed by the node.
    #[serde(default)]
    pub ledger: i64,
}

/// Health snapshot used to decide the PDB maxUnavailable value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncHealth {
    /// Node is actively syncing or catching up on ledgers.
    Syncing,
    /// Node is fully synced and stable.
    Synced,
    /// Health could not be determined; treat conservatively.
    Unknown,
}

impl SyncHealth {
    fn from_status(status: &StellarNodeStatus) -> Self {
        if status.syncing {
            SyncHealth::Syncing
        } else if status.synced {
            SyncHealth::Synced
        } else {
            SyncHealth::Unknown
        }
    }
}

/// Compute the desired `maxUnavailable` for a validator deployment group.
///
/// Returns `0` while any node in the group is syncing or of unknown health so
/// that `kubectl drain` cannot evict a validator mid-catch-up. Returns `1` only
/// when every node is fully synced and stable, and only if doing so would not
/// drop the group below its required quorum.
fn desired_max_unavailable(nodes: &[StellarNode], replicas: i32, min_quorum: i32) -> i32 {
    if nodes.is_empty() {
        return 0;
    }

    let all_synced = nodes
        .iter()
        .all(|n| SyncHealth::from_status(&n.status) == SyncHealth::Synced);

    if !all_synced {
        return 0;
    }

    // Guard against violating the minimum quorum required by the active
    // cluster topology: never allow more than `replicas - min_quorum`
    // simultaneous disruptions.
    let headroom = replicas.saturating_sub(min_quorum);
    if headroom < 1 {
        return 0;
    }

    1
}

/// Build the PodDisruptionBudget manifest for a validator deployment group.
fn build_pdb(
    namespace: &str,
    deployment_group: &str,
    max_unavailable: i32,
) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "policy/v1",
        "kind": "PodDisruptionBudget",
        "metadata": {
            "name": format!("stellar-validator-{deployment_group}"),
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "stellar-pdb-controller",
                "stellar.org/deployment-group": deployment_group,
            },
        },
        "spec": {
            "maxUnavailable": max_unavailable,
            "selector": {
                "matchLabels": {
                    "stellar.org/deployment-group": deployment_group,
                },
            },
        },
    })
}

/// Reconcile a single StellarNode: gather the health of its deployment group and
/// upsert the matching PodDisruptionBudget.
async fn reconcile(node: Arc<StellarNode>, client: Client) -> Result<Action> {
    let namespace = node
        .namespace()
        .context("StellarNode is missing a namespace")?;
    let group = node.spec.deployment_group.clone();

    let nodes_api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let group_nodes = nodes_api
        .list(&kube::api::ListParams::default())
        .await
        .context("failed to list StellarNodes")?
        .items
        .into_iter()
        .filter(|n| n.spec.deployment_group == group)
        .collect::<Vec<_>>();

    let max_unavailable = desired_max_unavailable(
        &group_nodes,
        node.spec.replicas,
        node.spec.min_quorum,
    );

    let pdb = build_pdb(&namespace, &group, max_unavailable);
    let pdb_api: Api<kube::api::DynamicObject> = Api::namespaced_with(
        client.clone(),
        &namespace,
        &kube::api::ApiResource::from_gvk(&kube::api::GroupVersionKind::gvk(
            "policy",
            "v1",
            "PodDisruptionBudget",
        )),
    );

    let name = format!("stellar-validator-{group}");
    pdb_api
        .patch(
            &name,
            &PatchParams::apply("stellar-pdb-controller").force(),
            &Patch::Apply(&pdb),
        )
        .await
        .context("failed to apply PodDisruptionBudget")?;

    info!(
        namespace = %namespace,
        group = %group,
        max_unavailable,
        "reconciled PodDisruptionBudget"
    );

    // Re-check periodically so PDBs track live sync health.
    Ok(Action::requeue(Duration::from_secs(30)))
}

fn error_policy(node: Arc<StellarNode>, err: &kube::Error, _ctx: Arc<()>) -> Action {
    error!(
        node = %node.name_any(),
        error = %err,
        "reconcile failed; retrying"
    );
    Action::requeue(Duration::from_secs(10))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let client = Client::try_default()
        .await
        .context("failed to create Kubernetes client")?;

    let nodes: Api<StellarNode> = Api::all(client.clone());

    info!("starting StellarNode PDB auto-tuning controller");

    Controller::new(nodes, watcher::Config::default())
        .run(reconcile, error_policy, Arc::new(()))
        .for_each(|res| async move {
            match res {
                Ok((obj, _)) => info!(node = %obj.name, "reconciled"),
                Err(err) => warn!(error = %err, "reconcile error"),
            }
        })
        .await;

    // Keep the process alive if the controller stream ends unexpectedly.
    sleep(Duration::from_secs(u64::MAX)).await;
    Ok(())
}

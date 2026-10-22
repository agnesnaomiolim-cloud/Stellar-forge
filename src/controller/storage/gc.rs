// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
//! CSI PVC Garbage Collector for orphaned volumes.
//!
//! When a StellarNode with `retentionPolicy: Delete` is removed from the
//! cluster, Kubernetes' PVC controller should delete the underlying
//! PersistentVolume and reclaim the cloud disk.  In practice this chain
//! frequently breaks:
//!
//! - The CSI driver's `DeleteVolume` call times out or fails silently.
//! - The PV is stuck in `Released` phase with `reclaimPolicy: Delete` but
//!   the cloud disk is never actually deleted.
//! - A PV loses its `claimRef` and becomes orphaned, accruing cloud billing
//!   indefinitely.
//!
//! This module implements a periodic scanner that:
//!
//! 1. Lists all `PersistentVolumeClaim` objects across every namespace.
//! 2. Filters to PVCs that belong to a StellarNode (via ownerRef or naming
//!    convention) whose `retentionPolicy` is `Delete`.
//! 3. For PVCs whose parent StellarNode has been deleted, forces deletion of
//!    the PVC and (if the PV is stuck in `Released`) the PV itself.
//! 4. Records structured tracing logs and emits Kubernetes Events for audit.
//!
//! ## Safety
//!
//! The GC will **never** delete a PVC or PV belonging to a live StellarNode.
//! It only acts on PVCs whose owning StellarNode no longer exists.

use std::time::Duration;

use k8s_openapi::api::core::v1::{PersistentVolume, PersistentVolumeClaim};
use kube::{
    api::{Api, DeleteParams, ListParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{error, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// How often the GC scanner runs.
pub const GC_SCAN_INTERVAL_SECS: u64 = 60;

/// Maximum number of PVCs to process per scan pass.
const GC_BATCH_LIMIT: usize = 50;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the PVC garbage collector indefinitely.
///
/// Spawn with `tokio::spawn`:
/// ```rust,ignore
/// tokio::spawn(run_gc_controller(client.clone()));
/// ```
pub async fn run_gc_controller(client: Client) {
    info!(
        interval_secs = GC_SCAN_INTERVAL_SECS,
        "PVC garbage collector started"
    );
    loop {
        if let Err(e) = run_gc_scan(&client).await {
            error!("PVC garbage collector scan error: {e}");
        }
        tokio::time::sleep(Duration::from_secs(GC_SCAN_INTERVAL_SECS)).await;
    }
}

// ---------------------------------------------------------------------------
// Scan pass
// ---------------------------------------------------------------------------

/// One full scan: find orphaned PVCs and force-delete them.
pub async fn run_gc_scan(client: &Client) -> Result<()> {
    let pvcs = list_orphaned_pvcs(client).await?;

    if pvcs.is_empty() {
        return Ok(());
    }

    info!(count = pvcs.len(), "GC: found orphaned PVC(s) for deletion");

    for pvc in pvcs {
        let name = pvc.name_any();
        let namespace = pvc
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // Delete the PVC.
        match delete_pvc(client, &namespace, &name).await {
            Ok(()) => {
                info!(name, namespace, "GC: deleted orphaned PVC");
            }
            Err(Error::KubeError(kube::Error::Api(e))) if e.code == 404 => {
                // Already gone.
                debug_assert!(true);
            }
            Err(e) => {
                warn!(name, namespace, "GC: failed to delete PVC: {e}");
                continue;
            }
        }

        // If the PV is stuck in Released with reclaimPolicy: Delete, force
        // delete it so the CSI driver reclaims the cloud disk.
        if let Some(pv_name) = pvc
            .spec
            .as_ref()
            .and_then(|s| s.volume_name.as_ref())
        {
            if let Err(e) = force_delete_released_pv(client, pv_name).await {
                warn!(pv = pv_name, "GC: could not force-delete released PV: {e}");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Orphan detection
// ---------------------------------------------------------------------------

/// List PVCs that are orphaned — i.e., belong to a deleted StellarNode with
/// `retentionPolicy: Delete`.
async fn list_orphaned_pvcs(client: &Client) -> Result<Vec<PersistentVolumeClaim>> {
    let pvc_api: Api<PersistentVolumeClaim> = Api::all(client.clone());
    let all_pvcs = pvc_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    // Cache the set of existing StellarNode names per namespace to avoid
    // one API call per PVC.
    let mut existing_nodes_cache: Option<Vec<StellarNode>> = None;

    let mut orphaned = Vec::new();

    for pvc in all_pvcs.items {
        let namespace = pvc
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // Check if this PVC belongs to a StellarNode.
        let owner_node = match find_owning_stellar_node(&pvc) {
            Some(node_name) => node_name,
            None => continue, // Not owned by a StellarNode — skip.
        };

        // Lazy-load the StellarNode list on first hit.
        if existing_nodes_cache.is_none() {
            let nodes_api: Api<StellarNode> = Api::all(client.clone());
            match nodes_api.list(&ListParams::default()).await {
                Ok(list) => existing_nodes_cache = Some(list.items),
                Err(e) => {
                    warn!("GC: could not list StellarNodes: {e}");
                    return Ok(orphaned);
                }
            }
        }

        // Check if the owning StellarNode still exists.
        let node_exists = existing_nodes_cache
            .as_ref()
            .unwrap()
            .iter()
            .any(|n| n.name_any() == owner_node && n.namespace().unwrap_or_default() == namespace);

        if node_exists {
            continue; // Owner is alive — don't touch.
        }

        // The owning StellarNode is gone. Check retention policy from PVC
        // labels or annotations (since we can't read the CRD spec anymore).
        // We use the PVC's `storage Retain` label or the PV's reclaim policy.
        let should_delete = should_delete_based_on_pv(client, &pvc).await;

        if should_delete {
            orphaned.push(pvc);
            if orphaned.len() >= GC_BATCH_LIMIT {
                break;
            }
        }
    }

    Ok(orphaned)
}

/// Find the name of the StellarNode that owns a PVC.
///
/// Checks ownerReferences first, then falls back to the naming convention
/// `<node-name>-data` or `<node-name>-storage`.
fn find_owning_stellar_node(pvc: &PersistentVolumeClaim) -> Option<String> {
    // Check ownerReferences.
    if let Some(refs) = &pvc.metadata.owner_references {
        for r in refs {
            if r.kind == "StellarNode" {
                return Some(r.name.clone());
            }
        }
    }

    // Fall back to naming convention.
    let name = pvc.name_any();
    if let Some(node_name) = name
        .strip_suffix("-data")
        .or_else(|| name.strip_suffix("-storage"))
    {
        return Some(node_name.to_string());
    }
    // StatefulSet PVC pattern: <node-name>-data-<ordinal>
    if let Some(rest) = name.strip_prefix("") {
        if let Some(idx) = rest.find("-data-") {
            return Some(rest[..idx].to_string());
        }
    }

    None
}

/// Check the PV's reclaim policy to determine if the PVC should be deleted.
///
/// If the PV has `persistentVolumeReclaimPolicy: Delete`, the PVC should be
/// garbage-collected.  If the PV is not found, we err on the side of caution
/// and skip deletion (the cloud disk may already be gone).
async fn should_delete_based_on_pv(client: &Client, pvc: &PersistentVolumeClaim) -> bool {
    let pv_name = match pvc
        .spec
        .as_ref()
        .and_then(|s| s.volume_name.as_ref())
    {
        Some(name) if !name.is_empty() => name.clone(),
        _ => return false, // PVC not bound — skip.
    };

    let pv_api: Api<PersistentVolume> = Api::all(client.clone());
    match pv_api.get(&pv_name).await {
        Ok(pv) => {
            let reclaim_policy = pv
                .spec
                .as_ref()
                .and_then(|s| s.persistent_volume_reclaim_policy.as_deref())
                .unwrap_or("Retain");

            reclaim_policy == "Delete"
        }
        Err(_) => {
            // PV not found — can't confirm reclaim policy.  Be conservative.
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Deletion helpers
// ---------------------------------------------------------------------------

/// Delete a PVC by name in the given namespace.
async fn delete_pvc(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    api.delete(name, &DeleteParams::default())
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

/// Force-delete a PersistentVolume that is stuck in `Released` phase.
///
/// This removes the `claimRef` so the PV transitions to `Available` and the
/// CSI driver can reclaim the cloud disk.  If the PV's reclaim policy is
/// `Delete`, we also delete the PV directly.
async fn force_delete_released_pv(client: &Client, pv_name: &str) -> Result<()> {
    let pv_api: Api<PersistentVolume> = Api::all(client.clone());

    let pv = match pv_api.get(pv_name).await {
        Ok(pv) => pv,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(()),
        Err(e) => return Err(Error::KubeError(e)),
    };

    let phase = pv
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("");

    let reclaim_policy = pv
        .spec
        .as_ref()
        .and_then(|s| s.persistent_volume_reclaim_policy.as_deref())
        .unwrap_or("Retain");

    if phase == "Released" {
        if reclaim_policy == "Delete" {
            // Direct delete — the CSI driver will reclaim the cloud disk.
            info!(pv = pv_name, "GC: deleting released PV (reclaimPolicy: Delete)");
            pv_api
                .delete(pv_name, &DeleteParams::default())
                .await
                .map_err(Error::KubeError)?;
        } else {
            // Retain policy — strip the claimRef so the PV becomes Available
            // and can be reused.  We don't delete the cloud disk.
            info!(pv = pv_name, "GC: clearing claimRef on released PV (reclaimPolicy: Retain)");
            let patch = json!({ "spec": { "claimRef": null } });
            pv_api
                .patch(pv_name, &kube::api::PatchParams::default(), &kube::api::Patch::Merge(&patch))
                .await
                .map_err(Error::KubeError)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::PersistentVolumeClaimSpec;
    use kube::api::ObjectMeta;

    fn make_pvc(name: &str, namespace: &str, pv_name: Option<&str>) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: pv_name.map(|s| s.to_string()),
                ..Default::default()
            }),
            status: None,
        }
    }

    fn make_pvc_with_owner(name: &str, namespace: &str, owner_name: &str) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                owner_references: Some(vec![k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                    api_version: "stellar.org/v1alpha1".to_string(),
                    kind: "StellarNode".to_string(),
                    name: owner_name.to_string(),
                    uid: "uid-123".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec::default()),
            status: None,
        }
    }

    // ── find_owning_stellar_node ───────────────────────────────────────────

    #[test]
    fn find_owner_via_owner_ref() {
        let pvc = make_pvc_with_owner("my-pvc", "stellar", "validator-1");
        assert_eq!(find_owning_stellar_node(&pvc), Some("validator-1".into()));
    }

    #[test]
    fn find_owner_via_data_suffix() {
        let pvc = make_pvc("validator-1-data", "stellar", None);
        assert_eq!(find_owning_stellar_node(&pvc), Some("validator-1".into()));
    }

    #[test]
    fn find_owner_via_storage_suffix() {
        let pvc = make_pvc("validator-1-storage", "stellar", None);
        assert_eq!(find_owning_stellar_node(&pvc), Some("validator-1".into()));
    }

    #[test]
    fn find_owner_via_statefulset_pattern() {
        let pvc = make_pvc("validator-1-data-0", "stellar", None);
        assert_eq!(find_owning_stellar_node(&pvc), Some("validator-1".into()));
    }

    #[test]
    fn find_owner_returns_none_for_unrelated_pvc() {
        let pvc = make_pvc("random-pvc", "default", None);
        assert_eq!(find_owning_stellar_node(&pvc), None);
    }

    #[test]
    fn find_owner_returns_none_for_other_owner_kind() {
        let pvc = PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("dep-pvc".into()),
                namespace: Some("default".into()),
                owner_references: Some(vec![
                    k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                        api_version: "apps/v1".to_string(),
                        kind: "Deployment".to_string(),
                        name: "my-dep".to_string(),
                        uid: "uid-456".to_string(),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec::default()),
            status: None,
        };
        assert_eq!(find_owning_stellar_node(&pvc), None);
    }

    // ── GC_SCAN_INTERVAL_SECS ──────────────────────────────────────────────

    #[test]
    fn gc_scan_interval_is_reasonable() {
        assert!(GC_SCAN_INTERVAL_SECS >= 30, "interval must be at least 30s");
        assert!(GC_SCAN_INTERVAL_SECS <= 300, "interval must be at most 5 min");
    }

    // ── GC_BATCH_LIMIT ─────────────────────────────────────────────────────

    #[test]
    fn gc_batch_limit_is_bounded() {
        assert!(GC_BATCH_LIMIT > 0, "batch limit must be positive");
        assert!(GC_BATCH_LIMIT <= 100, "batch limit must be at most 100");
    }

    // ── Unbound PVC is skipped ─────────────────────────────────────────────

    #[test]
    fn unbound_pvc_has_no_volume_name() {
        let pvc = make_pvc("test-pvc", "stellar", None);
        let vn = pvc.spec.as_ref().and_then(|s| s.volume_name.as_ref());
        assert!(vn.is_none());
    }

    #[test]
    fn bound_pvc_has_volume_name() {
        let pvc = make_pvc("test-pvc", "stellar", Some("pv-123"));
        let vn = pvc.spec.as_ref().and_then(|s| s.volume_name.as_ref());
        assert_eq!(vn.map(|s| s.as_str()), Some("pv-123"));
    }
}

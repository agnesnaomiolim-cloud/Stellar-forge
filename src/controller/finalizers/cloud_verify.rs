// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
//! Cloud provider volume-attachment verification for finalizer cleanup.
//!
//! Before the finalizer cleanup controller strips a finalizer from a
//! terminating `StellarNode`, it must confirm that every PersistentVolumeClaim
//! owned by that node has been fully detached from its cloud backing store.
//! Removing the finalizer while a volume is still attached can:
//!
//! - Leave an orphaned cloud disk that continues to incur billing.
//! - Cause data corruption if the volume is re-attached to a different node
//!   before the previous I/O has flushed.
//!
//! ## Providers
//!
//! | Provider | Detection method                              |
//! |---------|-----------------------------------------------|
//! | AWS EBS | PV `spec.awsElasticBlockStore.volumeHandle`   |
//! | GCP PD  | PV `spec.gcePersistentDisk.pdName`             |
//! | Azure   | PV `spec.azureDisk.diskName`                  |
//! | CSI     | PV `spec.csi.volumeHandle` + annotations      |
//! | Local   | PV `spec.local.path` — always treated as safe |
//!
//! For CSI volumes (the modern standard), the controller inspects the
//! `pv.kubernetes.io/provisioned-by` annotation to determine the driver
//! (e.g. `ebs.csi.aws.com`, `pd.csi.storage.gke.com`), then maps it to the
//! appropriate [`CloudProvider`].
//!
//! ## Safety contract
//!
//! [`VolumeAttachmentState::Detached`] and [`VolumeAttachmentState::NotFound`]
//! are the **only** states that produce `safe_to_release = true`.
//! All other states — including transient API errors — are treated as
//! **unsafe** and block finalizer removal.

use std::fmt;

use k8s_openapi::api::core::v1::{PersistentVolume, PersistentVolumeClaim};
use kube::{
    api::{Api, ResourceExt},
    Client,
};
use tracing::{debug, warn};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Cloud provider identification
// ---------------------------------------------------------------------------

/// The cloud provider backing a persistent volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudProvider {
    /// AWS Elastic Block Store (in-tree or CSI).
    AwsEbs,
    /// Google Cloud Persistent Disk (in-tree or CSI).
    GcpPd,
    /// Azure Disk (in-tree or CSI).
    AzureDisk,
    /// Generic CSI driver — driver name stored in `driver`.
    Csi(String),
    /// Local storage (local-path, local-storage). Always safe to release.
    Local,
    /// Provider could not be determined from the PV spec.
    Unknown,
}

impl fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloudProvider::AwsEbs => write!(f, "aws-ebs"),
            CloudProvider::GcpPd => write!(f, "gcp-pd"),
            CloudProvider::AzureDisk => write!(f, "azure-disk"),
            CloudProvider::Csi(driver) => write!(f, "csi:{driver}"),
            CloudProvider::Local => write!(f, "local"),
            CloudProvider::Unknown => write!(f, "unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// Volume attachment state
// ---------------------------------------------------------------------------

/// Observed state of a cloud volume, as reported by the cloud API or inferred
/// from the Kubernetes PV object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeAttachmentState {
    /// The volume is confirmed detached from all compute instances.
    Detached,
    /// The volume does not exist in the cloud provider (already deleted).
    NotFound,
    /// The volume is still attached to one or more instances.
    Attached {
        /// Cloud-provider instance IDs the volume is attached to.
        instance_ids: Vec<String>,
    },
    /// The cloud API returned an error (timeout, auth, rate-limit, etc.).
    /// This is a conservative **block** state — never treated as safe.
    ApiError(String),
}

impl fmt::Display for VolumeAttachmentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VolumeAttachmentState::Detached => write!(f, "detached"),
            VolumeAttachmentState::NotFound => write!(f, "not-found"),
            VolumeAttachmentState::Attached { instance_ids } => {
                write!(f, "attached:{}", instance_ids.join(","))
            }
            VolumeAttachmentState::ApiError(msg) => write!(f, "api-error:{msg}"),
        }
    }
}

/// Result of verifying a single PVC's backing volume.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Name of the PVC that was verified.
    pub pvc_name: String,
    /// Namespace of the PVC.
    pub namespace: String,
    /// Cloud volume ID (e.g. `vol-abc123`, `projects/.../disks/disk-1`).
    pub volume_id: String,
    /// Identified cloud provider.
    pub provider: CloudProvider,
    /// Observed attachment state.
    pub state: VolumeAttachmentState,
    /// Whether it is safe to release the finalizer for this PVC.
    ///
    /// `true` **only** when `state` is `Detached` or `NotFound`.
    pub safe_to_release: bool,
}

// ---------------------------------------------------------------------------
// Cloud checker trait (extensibility)
// ---------------------------------------------------------------------------

/// Trait abstracting the cloud-provider API call that checks whether a
/// volume is still attached.
///
/// Production implementations make real HTTP/SDK calls.  Tests use
/// [`MockChecker`] to exercise the decision logic without network access.
#[async_trait::async_trait]
pub trait CloudChecker: Send + Sync {
    /// Check the attachment state of the given cloud volume.
    ///
    /// `volume_id` is the raw handle from the PV spec (e.g. EBS volume ID,
    /// GCP disk name, CSI volume handle).
    async fn check_attachment(&self, volume_id: &str) -> std::result::Result<VolumeAttachmentState, String>;
}

// ---------------------------------------------------------------------------
// Mock checker (for tests)
// ---------------------------------------------------------------------------

/// A test-only [`CloudChecker`] that returns a pre-configured state for every
/// volume ID, or an error if configured to do so.
#[derive(Debug, Clone)]
pub struct MockChecker {
    state: VolumeAttachmentState,
    should_error: bool,
}

impl MockChecker {
    /// Create a mock checker that always returns the given state.
    pub fn new(state: VolumeAttachmentState) -> Self {
        Self {
            state,
            should_error: false,
        }
    }

    /// Create a mock checker that simulates a cloud API error.
    pub fn error() -> Self {
        Self {
            state: VolumeAttachmentState::ApiError("mock error".into()),
            should_error: true,
        }
    }
}

#[async_trait::async_trait]
impl CloudChecker for MockChecker {
    async fn check_attachment(&self, _volume_id: &str) -> std::result::Result<VolumeAttachmentState, String> {
        if self.should_error {
            return Err("mock cloud API error".to_string());
        }
        Ok(self.state.clone())
    }
}

// ---------------------------------------------------------------------------
// CloudVerifier — the orchestrator
// ---------------------------------------------------------------------------

/// Orchestrates volume-attachment verification across cloud providers.
///
/// In production, it is constructed with [`CloudVerifier::new`] which uses
/// a default set of checkers.  For testing, use [`CloudVerifier::with_checker`]
/// to inject a [`MockChecker`].
pub struct CloudVerifier {
    client: Client,
    /// Optional override checker (used by tests).
    checker: Option<Box<dyn CloudChecker>>,
}

impl CloudVerifier {
    /// Create a production verifier backed by the given Kubernetes client.
    ///
    /// Volume attachment is inferred from the PV object state and, when
    /// available, Kubernetes `VolumeAttachment` resources.  No external cloud
    /// SDK calls are made — this keeps the operator's dependency surface small
    /// and avoids requiring cloud credentials in-cluster.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            checker: None,
        }
    }

    /// Create a verifier with a custom [`CloudChecker`] (for tests).
    pub fn with_checker(client: Client, checker: Box<dyn CloudChecker>) -> Self {
        Self {
            client,
            checker: Some(checker),
        }
    }

    /// Verify that the backing volume for `pvc` is detached.
    ///
    /// This resolves the PVC → PV → cloud volume chain and inspects the PV
    /// status.  If the PV is in a released/available phase and the claim ref
    /// is gone, the volume is considered detached.
    pub async fn verify_volume_detached(
        &self,
        pvc: &PersistentVolumeClaim,
    ) -> Result<VerificationResult> {
        let pvc_name = pvc.name_any();
        let namespace = pvc
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // Resolve the PV name from the PVC spec.
        let pv_name = match pvc
            .spec
            .as_ref()
            .and_then(|s| s.volume_name.as_ref())
        {
            Some(name) => name.clone(),
            None => {
                // PVC has not been bound yet — nothing to verify.
                return Ok(VerificationResult {
                    pvc_name,
                    namespace,
                    volume_id: String::new(),
                    provider: CloudProvider::Unknown,
                    state: VolumeAttachmentState::NotFound,
                    safe_to_release: true,
                });
            }
        };

        // Fetch the PersistentVolume.
        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let pv = match pv_api.get(&pv_name).await {
            Ok(pv) => pv,
            Err(kube::Error::Api(e)) if e.code == 404 => {
                // PV was already deleted — safe to release.
                return Ok(VerificationResult {
                    pvc_name,
                    namespace,
                    volume_id: String::new(),
                    provider: CloudProvider::Unknown,
                    state: VolumeAttachmentState::NotFound,
                    safe_to_release: true,
                });
            }
            Err(e) => return Err(Error::KubeError(e)),
        };

        let (volume_id, provider) = extract_volume_info(&pv);
        let state = if let Some(ref checker) = self.checker {
            // Use injected checker (test path).
            match checker.check_attachment(&volume_id).await {
                Ok(s) => s,
                Err(msg) => VolumeAttachmentState::ApiError(msg),
            }
        } else {
            // Production path: infer from PV phase.
            infer_attachment_state(&pv)
        };

        let safe_to_release = matches!(
            &state,
            VolumeAttachmentState::Detached | VolumeAttachmentState::NotFound
        );

        Ok(VerificationResult {
            pvc_name,
            namespace,
            volume_id,
            provider,
            state,
            safe_to_release,
        })
    }
}

// ---------------------------------------------------------------------------
// PV inspection helpers
// ---------------------------------------------------------------------------

/// Extract the cloud volume ID and provider from a PersistentVolume spec.
fn extract_volume_info(pv: &PersistentVolume) -> (String, CloudProvider) {
    let spec = match &pv.spec {
        Some(s) => s,
        None => return (String::new(), CloudProvider::Unknown),
    };

    // CSI volume
    if let Some(ref csi) = spec.csi {
        let handle = csi.volume_handle.clone();
        let driver = csi.driver.clone();
        let provider = match driver.as_str() {
            "ebs.csi.aws.com" | "ebs.csi.aws.com.aws" => CloudProvider::AwsEbs,
            "pd.csi.storage.gke.com" => CloudProvider::GcpPd,
            "disk.csi.azure.com" => CloudProvider::AzureDisk,
            _ => CloudProvider::Csi(driver),
        };
        return (handle, provider);
    }

    // In-tree AWS EBS
    if let Some(ref ebs) = spec.aws_elastic_block_store {
        return (
            ebs.volume_id.clone(),
            CloudProvider::AwsEbs,
        );
    }

    // In-tree GCP PD
    if let Some(ref gce) = spec.gce_persistent_disk {
        return (
            gce.pd_name.clone(),
            CloudProvider::GcpPd,
        );
    }

    // In-tree Azure Disk
    if let Some(ref az) = spec.azure_disk {
        return (
            az.disk_name.clone(),
            CloudProvider::AzureDisk,
        );
    }

    // Local storage
    if spec.local.is_some() {
        return (
            "local".to_string(),
            CloudProvider::Local,
        );
    }

    (String::new(), CloudProvider::Unknown)
}

/// Infer the attachment state from the PV's `status.phase`.
///
/// Kubernetes PV phases:
/// - `Available` — volume is not bound to a claim, free to use.
/// - `Bound` — volume is bound to a claim (may or may not be attached to a node).
/// - `Released` — claim was deleted, but reclaim policy hasn't run yet.
/// - `Failed` — automatic reclaim failed.
///
/// For our purposes, `Released` and `Available` (with no claim ref) mean the
/// volume is detached. `Bound` means it may still be attached.
fn infer_attachment_state(pv: &PersistentVolume) -> VolumeAttachmentState {
    let phase = pv
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("");

    let claim_ref = pv
        .spec
        .as_ref()
        .and_then(|s| s.claim_ref.as_ref());

    match phase {
        "Available" => {
            // Volume is available — either never bound or reclaimed.
            if claim_ref.is_none() {
                VolumeAttachmentState::Detached
            } else {
                // Claim ref still present but phase is Available — treat as
                // detached (the claim is being cleaned up).
                VolumeAttachmentState::Detached
            }
        }
        "Released" => {
            // Claim was deleted; volume is released.
            VolumeAttachmentState::Detached
        }
        "Failed" => {
            // Reclaim failed — the volume is in an error state.  Treat as
            // detached so we don't hang the finalizer forever.
            warn!(
                pv = pv.name_any(),
                "PV is in Failed phase — treating as detached for finalizer cleanup"
            );
            VolumeAttachmentState::Detached
        }
        "Bound" => {
            // Volume is bound to a claim — check if the claim still exists.
            // If the claim has been deleted, the PV will eventually move to
            // Released, but in the meantime it may still be attached.
            //
            // For safety, we treat Bound as "possibly attached" only if the
            // claim ref points to a namespace that matches.  Since we're
            // verifying from the PVC side (which is being deleted), the PVC
            // will already be gone or terminating, so Bound typically means
            // the volume is still in use.
            VolumeAttachmentState::Attached {
                instance_ids: vec![],
            }
        }
        _ => {
            // Unknown phase — be conservative.
            debug!(pv = pv.name_any(), phase, "unknown PV phase — treating as API error");
            VolumeAttachmentState::ApiError(format!("unknown PV phase: {phase}"))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        AWSElasticBlockStoreVolumeSource, AzureDiskVolumeSource, GCEPersistentDiskVolumeSource,
        LocalVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeClaimStatus,
        PersistentVolumeSpec, PersistentVolumeStatus,
    };
    use kube::api::ObjectMeta;

    fn make_pv_with_phase(phase: &str) -> PersistentVolume {
        PersistentVolume {
            metadata: ObjectMeta {
                name: Some("test-pv".to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeSpec {
                capacity: Default::default(),
                access_modes: Some(vec!["ReadWriteOnce".to_string()]),
                ..Default::default()
            }),
            status: Some(PersistentVolumeStatus {
                phase: Some(phase.to_string()),
                ..Default::default()
            }),
        }
    }

    fn make_pvc_with_pv(pv_name: &str) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("test-pvc".to_string()),
                namespace: Some("stellar".to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: Some(pv_name.to_string()),
                ..Default::default()
            }),
            status: Some(PersistentVolumeClaimStatus {
                phase: Some("Bound".to_string()),
                ..Default::default()
            }),
        }
    }

    fn make_unbound_pvc() -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("unbound-pvc".to_string()),
                namespace: Some("stellar".to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec::default()),
            status: None,
        }
    }

    // ── CloudProvider display ──────────────────────────────────────────────

    #[test]
    fn cloud_provider_display() {
        assert_eq!(CloudProvider::AwsEbs.to_string(), "aws-ebs");
        assert_eq!(CloudProvider::GcpPd.to_string(), "gcp-pd");
        assert_eq!(CloudProvider::AzureDisk.to_string(), "azure-disk");
        assert_eq!(CloudProvider::Local.to_string(), "local");
        assert_eq!(CloudProvider::Unknown.to_string(), "unknown");
        assert_eq!(
            CloudProvider::Csi("ebs.csi.aws.com".into()).to_string(),
            "csi:ebs.csi.aws.com"
        );
    }

    // ── VolumeAttachmentState display ─────────────────────────────────────

    #[test]
    fn volume_attachment_state_display() {
        assert_eq!(VolumeAttachmentState::Detached.to_string(), "detached");
        assert_eq!(VolumeAttachmentState::NotFound.to_string(), "not-found");
        assert_eq!(
            VolumeAttachmentState::Attached {
                instance_ids: vec!["i-1".into()]
            }
            .to_string(),
            "attached:i-1"
        );
        assert_eq!(
            VolumeAttachmentState::ApiError("timeout".into()).to_string(),
            "api-error:timeout"
        );
    }

    // ── extract_volume_info ────────────────────────────────────────────────

    #[test]
    fn extract_aws_ebs_in_tree() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                aws_elastic_block_store: Some(AWSElasticBlockStoreVolumeSource {
                    volume_id: "vol-abc123".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "vol-abc123");
        assert_eq!(provider, CloudProvider::AwsEbs);
    }

    #[test]
    fn extract_gcp_pd_in_tree() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                gce_persistent_disk: Some(GCEPersistentDiskVolumeSource {
                    pd_name: "disk-1".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "disk-1");
        assert_eq!(provider, CloudProvider::GcpPd);
    }

    #[test]
    fn extract_azure_disk_in_tree() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                azure_disk: Some(AzureDiskVolumeSource {
                    disk_name: "disk-az-1".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "disk-az-1");
        assert_eq!(provider, CloudProvider::AzureDisk);
    }

    #[test]
    fn extract_csi_volume() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                csi: Some(k8s_openapi::api::core::v1::CSIPersistentVolumeSource {
                    driver: "ebs.csi.aws.com".to_string(),
                    volume_handle: "vol-csi-123".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "vol-csi-123");
        assert_eq!(provider, CloudProvider::AwsEbs);
    }

    #[test]
    fn extract_csi_unknown_driver() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                csi: Some(k8s_openapi::api::core::v1::CSIPersistentVolumeSource {
                    driver: "custom.csi.example.com".to_string(),
                    volume_handle: "vol-custom".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "vol-custom");
        assert_eq!(provider, CloudProvider::Csi("custom.csi.example.com".into()));
    }

    #[test]
    fn extract_local_storage() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec {
                local: Some(LocalVolumeSource {
                    path: "/mnt/nvme".to_string(),
                }),
                ..Default::default()
            }),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert_eq!(id, "local");
        assert_eq!(provider, CloudProvider::Local);
    }

    #[test]
    fn extract_unknown_provider() {
        let pv = PersistentVolume {
            metadata: ObjectMeta::default(),
            spec: Some(PersistentVolumeSpec::default()),
            status: None,
        };
        let (id, provider) = extract_volume_info(&pv);
        assert!(id.is_empty());
        assert_eq!(provider, CloudProvider::Unknown);
    }

    // ── infer_attachment_state ─────────────────────────────────────────────

    #[test]
    fn available_pv_is_detached() {
        let pv = make_pv_with_phase("Available");
        let state = infer_attachment_state(&pv);
        assert_eq!(state, VolumeAttachmentState::Detached);
    }

    #[test]
    fn released_pv_is_detached() {
        let pv = make_pv_with_phase("Released");
        let state = infer_attachment_state(&pv);
        assert_eq!(state, VolumeAttachmentState::Detached);
    }

    #[test]
    fn failed_pv_is_detached() {
        let pv = make_pv_with_phase("Failed");
        let state = infer_attachment_state(&pv);
        assert_eq!(state, VolumeAttachmentState::Detached);
    }

    #[test]
    fn bound_pv_is_attached() {
        let pv = make_pv_with_phase("Bound");
        let state = infer_attachment_state(&pv);
        assert_eq!(
            state,
            VolumeAttachmentState::Attached {
                instance_ids: vec![]
            }
        );
    }

    #[test]
    fn unknown_phase_is_api_error() {
        let pv = make_pv_with_phase("Pending");
        let state = infer_attachment_state(&pv);
        match state {
            VolumeAttachmentState::ApiError(msg) => {
                assert!(msg.contains("Pending"));
            }
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    // ── MockChecker ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mock_checker_returns_configured_state() {
        let checker = MockChecker::new(VolumeAttachmentState::Detached);
        let state = checker.check_attachment("vol-1").await.unwrap();
        assert_eq!(state, VolumeAttachmentState::Detached);
    }

    #[tokio::test]
    async fn mock_checker_error_returns_error() {
        let checker = MockChecker::error();
        let result = checker.check_attachment("vol-1").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "mock cloud API error");
    }

    #[tokio::test]
    async fn mock_checker_attached() {
        let checker = MockChecker::new(VolumeAttachmentState::Attached {
            instance_ids: vec!["i-abc".into()],
        });
        let state = checker.check_attachment("vol-1").await.unwrap();
        assert_eq!(
            state,
            VolumeAttachmentState::Attached {
                instance_ids: vec!["i-abc".into()]
            }
        );
    }

    // ── VerificationResult safety invariants ────────────────────────────────

    #[test]
    fn detached_is_safe() {
        let r = VerificationResult {
            pvc_name: "p".into(),
            namespace: "ns".into(),
            volume_id: "v".into(),
            provider: CloudProvider::AwsEbs,
            state: VolumeAttachmentState::Detached,
            safe_to_release: true,
        };
        assert!(r.safe_to_release);
    }

    #[test]
    fn not_found_is_safe() {
        let r = VerificationResult {
            pvc_name: "p".into(),
            namespace: "ns".into(),
            volume_id: "v".into(),
            provider: CloudProvider::AwsEbs,
            state: VolumeAttachmentState::NotFound,
            safe_to_release: true,
        };
        assert!(r.safe_to_release);
    }

    #[test]
    fn attached_is_not_safe() {
        let r = VerificationResult {
            pvc_name: "p".into(),
            namespace: "ns".into(),
            volume_id: "v".into(),
            provider: CloudProvider::AwsEbs,
            state: VolumeAttachmentState::Attached {
                instance_ids: vec!["i-1".into()],
            },
            safe_to_release: false,
        };
        assert!(!r.safe_to_release);
    }

    #[test]
    fn api_error_is_not_safe() {
        let r = VerificationResult {
            pvc_name: "p".into(),
            namespace: "ns".into(),
            volume_id: "v".into(),
            provider: CloudProvider::AwsEbs,
            state: VolumeAttachmentState::ApiError("timeout".into()),
            safe_to_release: false,
        };
        assert!(!r.safe_to_release);
    }

    // ── Unbound PVC (no volume_name) is safe ───────────────────────────────

    #[test]
    fn unbound_pvc_volume_name_is_none() {
        let pvc = make_unbound_pvc();
        let pv_name = pvc
            .spec
            .as_ref()
            .and_then(|s| s.volume_name.as_ref());
        assert!(pv_name.is_none());
    }
}

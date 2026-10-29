// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Multi-Cluster Snapshot Synchronization subsystem.
//!
//! Provides two submodules:
//!
//! - [`verifier`]: SHA-256 integrity checking for snapshot archives
//! - [`reconciler`]: automated download, verify, extract, and bootstrap loop
//!
//! # Overview
//!
//! Secondary cluster nodes can be bootstrapped from recent ledger snapshots
//! stored in S3-compatible cloud storage. The reconciler discovers the latest
//! archive, streams it to disk, verifies its integrity via SHA-256, extracts
//! it atomically, and marks the node as bootstrapped via a sentinel file.

pub mod reconciler;
pub mod verifier;

pub use reconciler::{ReconcileOutcome, SnapshotReconciler, SnapshotReconcilerConfig, SnapshotRef};
pub use verifier::{compute_sha256_sync, parse_sha256_sidecar, verify_file, VerificationResult};

// Preserve the public CSI snapshot API alongside verified archive bootstrap.
pub use super::csi_snapshot::{
    create_pre_upgrade_snapshot, get_volume_snapshot_readiness, reconcile_snapshot,
    request_db_flush, VolumeSnapshotReadiness,
};

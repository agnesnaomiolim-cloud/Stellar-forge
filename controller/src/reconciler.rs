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

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VolumeSnapshotRecord {
    pub snapshot_name: String,
    pub pvc_name: String,
    pub volume_snapshot_class: String,
    pub target_image_tag: String,
    pub created_at_ms: u64,
    pub ready_to_use: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UpgradeRolloutAction {
    NoUpgradeNeeded,
    CreateVolumeSnapshot(VolumeSnapshotRecord),
    WaitForSnapshotReady {
        snapshot_name: String,
    },
    ProceedWithRollingUpgrade {
        verified_snapshot: String,
        new_image: String,
    },
}

pub struct PreUpgradeSnapshotReconciler {
    pub snapshot_class_name: String,
    pub max_retained_snapshots: usize,
    pub snapshots: Vec<VolumeSnapshotRecord>,
}

impl PreUpgradeSnapshotReconciler {
    pub fn new(snapshot_class_name: impl Into<String>, max_retained_snapshots: usize) -> Self {
        Self {
            snapshot_class_name: snapshot_class_name.into(),
            max_retained_snapshots: max_retained_snapshots.max(1),
            snapshots: Vec::new(),
        }
    }

    /// Reconciles a StellarNode image change by gating the StatefulSet rolling update
    /// until a CSI VolumeSnapshot (`snapshot.storage.k8s.io/v1`) has `readyToUse == true`.
    pub fn reconcile_upgrade(
        &mut self,
        node_name: &str,
        pvc_name: &str,
        current_running_image: &str,
        desired_spec_image: &str,
        now_ms: u64,
    ) -> UpgradeRolloutAction {
        if current_running_image == desired_spec_image {
            return UpgradeRolloutAction::NoUpgradeNeeded;
        }

        let sanitized_tag = desired_spec_image
            .rsplit(':')
            .next()
            .unwrap_or("upgrade")
            .replace(['.', '/', ':'], "-");
        let expected_snap_name = format!("{}-pre-upgrade-{}", node_name, sanitized_tag);

        if let Some(existing) = self
            .snapshots
            .iter()
            .find(|s| s.snapshot_name == expected_snap_name)
        {
            if existing.ready_to_use {
                let verified_name = existing.snapshot_name.clone();
                self.prune_old_snapshots();
                return UpgradeRolloutAction::ProceedWithRollingUpgrade {
                    verified_snapshot: verified_name,
                    new_image: desired_spec_image.to_string(),
                };
            } else {
                return UpgradeRolloutAction::WaitForSnapshotReady {
                    snapshot_name: existing.snapshot_name.clone(),
                };
            }
        }

        let record = VolumeSnapshotRecord {
            snapshot_name: expected_snap_name,
            pvc_name: pvc_name.to_string(),
            volume_snapshot_class: self.snapshot_class_name.clone(),
            target_image_tag: desired_spec_image.to_string(),
            created_at_ms: now_ms,
            ready_to_use: false,
        };
        self.snapshots.push(record.clone());
        UpgradeRolloutAction::CreateVolumeSnapshot(record)
    }

    pub fn mark_snapshot_ready(&mut self, snapshot_name: &str) -> bool {
        if let Some(snap) = self
            .snapshots
            .iter_mut()
            .find(|s| s.snapshot_name == snapshot_name)
        {
            snap.ready_to_use = true;
            true
        } else {
            false
        }
    }

    pub fn prune_old_snapshots(&mut self) -> usize {
        if self.snapshots.len() <= self.max_retained_snapshots {
            return 0;
        }
        self.snapshots.sort_by_key(|s| s.created_at_ms);
        let remove_count = self.snapshots.len() - self.max_retained_snapshots;
        self.snapshots.drain(0..remove_count);
        remove_count
    }
}

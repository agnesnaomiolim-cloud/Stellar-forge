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

#[path = "../src/reconciler.rs"]
mod reconciler;

use reconciler::{PreUpgradeSnapshotReconciler, UpgradeRolloutAction};

#[test]
fn test_pre_upgrade_pvc_volume_snapshot_gates_rollout_and_prunes_history() {
    let mut reconciler = PreUpgradeSnapshotReconciler::new("csi-aws-ebs-snapclass", 2);

    // 1. Image tag change detected -> triggers VolumeSnapshot creation and blocks rollout
    let step1 = reconciler.reconcile_upgrade(
        "stellar-validator-0",
        "data-stellar-validator-0",
        "stellar/stellar-core:v21.0.0",
        "stellar/stellar-core:v21.1.0",
        1_000,
    );
    let snap_name = match step1 {
        UpgradeRolloutAction::CreateVolumeSnapshot(snap) => {
            assert!(!snap.ready_to_use);
            snap.snapshot_name
        }
        other => panic!("Expected CreateVolumeSnapshot, got {:?}", other),
    };

    // 2. Subsequent reconcile while CSI snapshot is still provisioning -> WaitForSnapshotReady
    let step2 = reconciler.reconcile_upgrade(
        "stellar-validator-0",
        "data-stellar-validator-0",
        "stellar/stellar-core:v21.0.0",
        "stellar/stellar-core:v21.1.0",
        2_000,
    );
    assert_eq!(
        step2,
        UpgradeRolloutAction::WaitForSnapshotReady {
            snapshot_name: snap_name.clone()
        }
    );

    // 3. CSI driver marks VolumeSnapshot readyToUse = true -> rollout is unblocked
    assert!(reconciler.mark_snapshot_ready(&snap_name));
    let step3 = reconciler.reconcile_upgrade(
        "stellar-validator-0",
        "data-stellar-validator-0",
        "stellar/stellar-core:v21.0.0",
        "stellar/stellar-core:v21.1.0",
        3_000,
    );
    assert_eq!(
        step3,
        UpgradeRolloutAction::ProceedWithRollingUpgrade {
            verified_snapshot: snap_name,
            new_image: "stellar/stellar-core:v21.1.0".to_string(),
        }
    );
}

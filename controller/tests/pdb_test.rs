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
#[path = "../src/pdb/mod.rs"]
mod pdb;

use pdb::calculator::{ValidatorGroupHealth, ValidatorSyncPhase};
use pdb::reconciler::{reconcile_validator_pdb, PdbReconcileOutcome};

#[test]
fn test_pdb_blocks_drain_during_sync_and_allows_when_synced_above_quorum() {
    let syncing = ValidatorGroupHealth {
        group_name: "validator-tier1".to_string(),
        total_replicas: 4,
        min_quorum_replicas: 3,
        synced_replicas: 4,
        phase: ValidatorSyncPhase::CatchingUp,
    };

    let (calc_syncing, outcome_syncing) = reconcile_validator_pdb("stellar", &syncing, 0);
    assert_eq!(calc_syncing.max_unavailable, 0);
    assert!(!calc_syncing.drain_allowed);
    assert_eq!(
        outcome_syncing,
        PdbReconcileOutcome::Applied(pdb::reconciler::PodDisruptionBudgetManifest {
            api_version: "policy/v1".to_string(),
            kind: "PodDisruptionBudget".to_string(),
            name: "validator-tier1-pdb".to_string(),
            namespace: "stellar".to_string(),
            match_label_group: "validator-tier1".to_string(),
            max_unavailable: 0,
        })
    );

    let synced = ValidatorGroupHealth {
        group_name: "validator-tier1".to_string(),
        total_replicas: 4,
        min_quorum_replicas: 3,
        synced_replicas: 4,
        phase: ValidatorSyncPhase::Synced,
    };
    let (calc_synced, _) = reconcile_validator_pdb("stellar", &synced, 0);
    assert_eq!(calc_synced.max_unavailable, 1);
    assert!(calc_synced.drain_allowed);

    // Quorum violation guard when 2 pods are already evicting
    let (_, outcome_violation) = reconcile_validator_pdb("stellar", &synced, 2);
    assert_eq!(
        outcome_violation,
        PdbReconcileOutcome::RejectedQuorumViolation(
            "projected_healthy_below_min_quorum".to_string()
        )
    );
}

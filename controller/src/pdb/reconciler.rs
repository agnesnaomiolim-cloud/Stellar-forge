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
use crate::pdb::calculator::{
    calculate_max_unavailable, PdbCalculationResult, ValidatorGroupHealth,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodDisruptionBudgetManifest {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub match_label_group: String,
    pub max_unavailable: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdbReconcileOutcome {
    Applied(PodDisruptionBudgetManifest),
    RejectedQuorumViolation(String),
}

pub fn reconcile_validator_pdb(
    namespace: &str,
    health: &ValidatorGroupHealth,
    current_evicting_pods: u32,
) -> (PdbCalculationResult, PdbReconcileOutcome) {
    let calc = calculate_max_unavailable(health);

    // Block update or eviction if active evictions would drop healthy nodes below quorum
    let projected_healthy = health.synced_replicas.saturating_sub(current_evicting_pods);
    if projected_healthy < health.min_quorum_replicas {
        return (
            calc,
            PdbReconcileOutcome::RejectedQuorumViolation(
                "projected_healthy_below_min_quorum".to_string(),
            ),
        );
    }

    let manifest = PodDisruptionBudgetManifest {
        api_version: "policy/v1".to_string(),
        kind: "PodDisruptionBudget".to_string(),
        name: format!("{}-pdb", health.group_name),
        namespace: namespace.to_string(),
        match_label_group: health.group_name.clone(),
        max_unavailable: calc.max_unavailable,
    };

    (calc, PdbReconcileOutcome::Applied(manifest))
}

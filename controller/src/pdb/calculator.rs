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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValidatorSyncPhase {
    Bootstrapping,
    CatchingUp,
    Synced,
    Degraded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatorGroupHealth {
    pub group_name: String,
    pub total_replicas: u32,
    pub min_quorum_replicas: u32,
    pub synced_replicas: u32,
    pub phase: ValidatorSyncPhase,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PdbCalculationResult {
    pub max_unavailable: u32,
    pub drain_allowed: bool,
    pub reason: String,
}

/// Computes `maxUnavailable` for a validator group's `PodDisruptionBudget`
/// based on live SCP sync status and minimum quorum constraints.
pub fn calculate_max_unavailable(health: &ValidatorGroupHealth) -> PdbCalculationResult {
    if health.phase != ValidatorSyncPhase::Synced {
        return PdbCalculationResult {
            max_unavailable: 0,
            drain_allowed: false,
            reason: format!("node_not_synced:{:?}", health.phase),
        };
    }

    if health.synced_replicas <= health.min_quorum_replicas
        || health.total_replicas <= health.min_quorum_replicas
    {
        return PdbCalculationResult {
            max_unavailable: 0,
            drain_allowed: false,
            reason: "quorum_floor_reached".to_string(),
        };
    }

    PdbCalculationResult {
        max_unavailable: 1,
        drain_allowed: true,
        reason: "synced_with_quorum_headroom".to_string(),
    }
}

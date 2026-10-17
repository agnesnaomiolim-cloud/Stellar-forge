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

pub const DEFAULT_SCALE_UP_REACTION_SLA_MS: u64 = 15_000;
pub const DEFAULT_SCALE_DOWN_STABILIZATION_MS: u64 = 300_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SorobanRpcMetricsSnapshot {
    pub simulate_tx_queue_depth: u32,
    pub wasm_instantiation_p95_ms: u64,
    pub current_replicas: u32,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AutoscalerPolicy {
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub target_queue_depth_per_pod: u32,
    pub target_wasm_p95_ms: u64,
    pub scale_down_stabilization_ms: u64,
}

impl Default for AutoscalerPolicy {
    fn default() -> Self {
        Self {
            min_replicas: 2,
            max_replicas: 16,
            target_queue_depth_per_pod: 25,
            target_wasm_p95_ms: 150,
            scale_down_stabilization_ms: DEFAULT_SCALE_DOWN_STABILIZATION_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ScalingDirection {
    ScaleUp,
    ScaleDown,
    HoldCooldown,
    NoChange,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScalingDecision {
    pub desired_replicas: u32,
    pub direction: ScalingDirection,
    pub reason: String,
}

pub struct SorobanRpcAutoscaler {
    pub policy: AutoscalerPolicy,
    pub last_scale_up_timestamp_ms: Option<u64>,
    pub last_scale_down_timestamp_ms: Option<u64>,
}

impl SorobanRpcAutoscaler {
    pub fn new(policy: AutoscalerPolicy) -> Self {
        Self {
            policy,
            last_scale_up_timestamp_ms: None,
            last_scale_down_timestamp_ms: None,
        }
    }

    pub fn evaluate(&mut self, metrics: &SorobanRpcMetricsSnapshot) -> ScalingDecision {
        let current = metrics
            .current_replicas
            .clamp(self.policy.min_replicas, self.policy.max_replicas);

        // Compute desired replicas from queue depth ratio
        let by_queue = if self.policy.target_queue_depth_per_pod > 0 {
            metrics
                .simulate_tx_queue_depth
                .div_ceil(self.policy.target_queue_depth_per_pod)
        } else {
            current
        };

        // Compute desired replicas from WASM instantiation p95 latency ratio
        let by_latency = if self.policy.target_wasm_p95_ms > 0
            && metrics.wasm_instantiation_p95_ms > self.policy.target_wasm_p95_ms
        {
            let numer = (current as u64).saturating_mul(metrics.wasm_instantiation_p95_ms);
            numer.div_ceil(self.policy.target_wasm_p95_ms) as u32
        } else if metrics.wasm_instantiation_p95_ms <= self.policy.target_wasm_p95_ms / 2 {
            (current / 2).max(self.policy.min_replicas)
        } else {
            current
        };

        let raw_desired = by_queue
            .max(by_latency)
            .clamp(self.policy.min_replicas, self.policy.max_replicas);

        if raw_desired > current {
            self.last_scale_up_timestamp_ms = Some(metrics.timestamp_ms);
            ScalingDecision {
                desired_replicas: raw_desired,
                direction: ScalingDirection::ScaleUp,
                reason: format!(
                    "queue_depth={} wasm_p95_ms={} exceeded target thresholds",
                    metrics.simulate_tx_queue_depth, metrics.wasm_instantiation_p95_ms
                ),
            }
        } else if raw_desired < current {
            let last_change = self
                .last_scale_up_timestamp_ms
                .into_iter()
                .chain(self.last_scale_down_timestamp_ms)
                .max();
            if let Some(last_ts) = last_change {
                if metrics.timestamp_ms.saturating_sub(last_ts)
                    < self.policy.scale_down_stabilization_ms
                {
                    return ScalingDecision {
                        desired_replicas: current,
                        direction: ScalingDirection::HoldCooldown,
                        reason: "within 5-minute stabilization cooldown window".to_string(),
                    };
                }
            }
            self.last_scale_down_timestamp_ms = Some(metrics.timestamp_ms);
            ScalingDecision {
                desired_replicas: raw_desired,
                direction: ScalingDirection::ScaleDown,
                reason: "load subsided past 5-minute stabilization window".to_string(),
            }
        } else {
            ScalingDecision {
                desired_replicas: current,
                direction: ScalingDirection::NoChange,
                reason: "metrics within target band".to_string(),
            }
        }
    }
}

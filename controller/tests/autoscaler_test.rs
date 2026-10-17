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

#[path = "../src/autoscaler.rs"]
mod autoscaler;

use autoscaler::{
    AutoscalerPolicy, ScalingDirection, SorobanRpcAutoscaler, SorobanRpcMetricsSnapshot,
};

#[test]
fn test_soroban_rpc_autoscaler_fast_scale_up_and_5min_cooldown() {
    assert_eq!(autoscaler::DEFAULT_SCALE_UP_REACTION_SLA_MS, 15_000);
    let mut autoscaler = SorobanRpcAutoscaler::new(AutoscalerPolicy::default());

    // 1. Spike in simulateTransaction queue depth (120 queued, 25 target/pod) -> scales up immediately (<15s) to 5 pods
    let spike = SorobanRpcMetricsSnapshot {
        simulate_tx_queue_depth: 120,
        wasm_instantiation_p95_ms: 450,
        current_replicas: 2,
        timestamp_ms: 10_000,
    };
    let up_decision = autoscaler.evaluate(&spike);
    assert_eq!(up_decision.direction, ScalingDirection::ScaleUp);
    assert_eq!(up_decision.desired_replicas, 6);

    // 2. Load drops 60 seconds later -> blocked by 5-minute (300,000ms) stabilization window
    let early_drop = SorobanRpcMetricsSnapshot {
        simulate_tx_queue_depth: 10,
        wasm_instantiation_p95_ms: 40,
        current_replicas: 6,
        timestamp_ms: 70_000,
    };
    let hold_decision = autoscaler.evaluate(&early_drop);
    assert_eq!(hold_decision.direction, ScalingDirection::HoldCooldown);
    assert_eq!(hold_decision.desired_replicas, 6);

    // 3. After 5 minutes (315,000ms) -> safely scales down
    let post_cooldown_drop = SorobanRpcMetricsSnapshot {
        simulate_tx_queue_depth: 10,
        wasm_instantiation_p95_ms: 40,
        current_replicas: 6,
        timestamp_ms: 315_000,
    };
    let down_decision = autoscaler.evaluate(&post_cooldown_drop);
    assert_eq!(down_decision.direction, ScalingDirection::ScaleDown);
    assert_eq!(down_decision.desired_replicas, 3);
}

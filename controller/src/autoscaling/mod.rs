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
// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Latency-driven autoscaling for Soroban RPC replicas.
//!
//! - [`wasm_metrics`] — histogram tracking WASM host-function execution time,
//!   exposing p50/p95/p99 as the `soroban_wasm_latency_ms` metric.
//! - [`hpa_adapter`] — custom-metrics adapter that serves the metric to the
//!   Kubernetes HPA, plus the sustained-breach / cool-down scaling policy.

pub mod hpa_adapter;
pub mod wasm_metrics;
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
//! Gateway module: HTTP proxy middleware for Soroban RPC nodes.
//!
//! Provides request rate-limiting, IP tracking, and dynamic throttling based
//! on live CPU utilisation metrics.
//!
//! # Modules
//!
//! - [`ratelimit`]: sliding-window per-IP tracker + CPU-aware engine

pub mod ratelimit;

pub use ratelimit::{
    extract_client_ip, EngineMetrics, RateLimitConfig, RateLimitDecision, RateLimitEngine,
    SlidingWindowTracker,
};

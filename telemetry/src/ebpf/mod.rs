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

//! eBPF-based low-overhead network telemetry for the Stellar Kubernetes Operator.
//!
//! This module implements the feature described in issue #17:
//!
//! > Develop an eBPF probe (using Aya-rs) that attaches to the host's
//! > networking stack to monitor ingress/egress connections to Stellar core
//! > pods, stream aggregated connection metrics (packet drops, latency,
//! > connection resets) back to the main operator telemetry engine, and map
//! > raw IP addresses back to Kubernetes Pod identities dynamically.
//!
//! # Module layout
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`probe`] | Kernel-side eBPF program definitions + [`ProbeSimulator`] fallback |
//! | [`collector`] | Poll loop, IP-to-pod mapping, Prometheus recording |
//! | [`metrics`] | Prometheus metric types and label definitions |
//!
//! # Overhead target
//!
//! The collector is designed for **< 1% CPU overhead** at Stellar Core
//! throughput (≈ 5,000 connections/s).  The design achieves this by:
//!
//! - Batched event draining (up to 4,096 events per poll).
//! - Lock-free Prometheus counter increments via `AtomicU64`.
//! - Amortized IP-to-pod map lookups (read-lock only on hot path).
//! - 100 ms poll interval avoids busy-waiting.
//!
//! See `telemetry/benches/ebpf_overhead.rs` for the performance benchmark.
//!
//! # Quick start
//!
//! ```no_run
//! use stellar_telemetry::ebpf::collector::{EbpfCollector, CollectorConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let collector = std::sync::Arc::new(
//!         EbpfCollector::new(CollectorConfig::default()).await?
//!     );
//!     // Serve metrics on /ebpf/metrics endpoint alongside existing exporter.
//!     let c = collector.clone();
//!     tokio::spawn(async move { c.run().await.ok() });
//!     println!("{}", collector.render_metrics());
//!     Ok(())
//! }
//! ```

pub mod collector;
pub mod metrics;
pub mod probe;

// Re-export the most commonly used types for ergonomic access.
pub use collector::{CollectorConfig, EbpfCollector, PodIdentity, PodMap};
pub use metrics::EbpfMetrics;
pub use probe::{ConnEvent, Direction, DropReason, ProbeConfig, ProbeHandle, ProbeSimulator};

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
//! Stellar Telemetry — PromQL Metrics Exporter and Proxy for Stellar Infrastructure
//!
//! This crate provides:
//! - An asynchronous log parser and Prometheus metrics exporter for monitoring
//!   Soroban smart contract CPU and memory consumption.
//! - An intelligent rate-limiter proxy that sits between Prometheus and the
//!   Stellar Core `/metrics` endpoint, protecting high-throughput nodes from
//!   CPU spikes caused by frequent scrapes.
//!
//! # Quick start — gas exporter
//!
//! ```no_run
//! use stellar_telemetry::exporter::run_exporter;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     run_exporter("0.0.0.0:9100").await
//! }
//! ```
//!
//! # Quick start — metrics proxy
//!
//! ```no_run
//! use stellar_telemetry::proxy::run_from_env;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_from_env().await
//! }
//! ```
//!
//! # Modules
//!
//! - [`parser`]  — Zero-copy async log parser for Soroban RPC invocation streams.
//! - [`exporter`] — Prometheus metrics exporter with an HTTP `/metrics` endpoint.
//! - [`proxy`]   — Intelligent rate-limiter proxy for the Stellar Core metrics endpoint.
//! - [`ebpf`]    — Optional Aya-rs socket telemetry with graceful fallback.
//! - [`metrics`] — Prometheus text for eBPF/fallback network counters.

pub mod ebpf;
pub mod exporter;
pub mod metrics;
pub mod parser;
pub mod profiling;
pub mod proxy;
pub mod server;
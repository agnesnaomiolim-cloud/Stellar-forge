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
//! Host function benchmarking and calibration module for Soroban fee modeling.
//!
//! Benchmarks core Soroban host functions (e.g. cryptographic hashing, ledger reads/writes,
//! curve operations) on isolated/dedicated CPU cores to avoid interfering with live
//! Byzantine Fault Tolerant consensus execution.

pub mod exporter;
pub mod host_functions;

pub use exporter::{CalibrationExporter, CalibratorConfig, MetricsState};
pub use host_functions::{
    benchmark_all_host_functions, BenchmarkResult, FeeConfiguration, HardwareRiskLevel,
    HostFunctionBenchmark, HostFunctionCalibrator, HostFunctionType,
};
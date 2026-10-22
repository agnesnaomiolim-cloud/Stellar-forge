// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
//! Zero-copy log parser for Soroban RPC invocation records.
//!
//! [`InvocationRecord`] is the lightweight struct consumed by
//! [`crate::exporter::MetricsExporter`] to record Prometheus metrics.

/// A parsed Soroban RPC invocation record.
///
/// All string fields are borrowed from the source line to avoid allocations
/// on the hot path.
#[derive(Debug, Clone)]
pub struct InvocationRecord<'a> {
    /// Contract ID (hex-encoded).
    pub contract_id: &'a str,
    /// Host function name (e.g. `InvokeContractFunction`).
    pub host_function: &'a str,
    /// CPU instructions consumed.
    pub cpu_instructions: u64,
    /// Peak memory bytes consumed.
    pub memory_bytes: u64,
    /// Wasm execution duration in microseconds.
    pub wasm_execution_duration_us: u64,
    /// Storage fee in stroops.
    pub storage_fee_stroops: u64,
    /// Whether the invocation succeeded.
    pub success: &'a str,
}

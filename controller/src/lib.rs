// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Stellar-K8s controller crate.
//!
//! This crate provides the deployment pipeline and admission webhook modules
//! used by the Stellar-K8s operator.  It is intentionally kept lean so it can
//! be compiled quickly and linked into the operator binary without pulling in
//! heavy optional dependencies.
//!
//! # Modules
//!
//! - [`deployment`] — StellarNode deployment pipeline, including the
//!   [`deployment::optimizer`] WASM bytecode optimizer.
//! - [`quorum`] — Quorum-set graph analysis utilities.
//! - [`snapshots`] — Ephemeral Kubernetes storage snapshot controller (issue #243):
//!   VolumeSnapshot watcher, fsync coordination with Stellar Core, cloud-provider
//!   API adapters (AWS EBS, GCP PD), and PVC auto-generation for rapid horizontal
//!   scaling.
//! - [`webhook`] — Admission webhook handlers, including the
//!   [`webhook::wasm_mutator`] MutatingAdmissionWebhook for WASM optimization.

pub mod deployment;
pub mod quorum;
pub mod snapshots;
pub mod webhook;

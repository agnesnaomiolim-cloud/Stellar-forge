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
//! - [`apis`] — Custom resource validation and API schema utilities.
//! - [`deployment`] — StellarNode deployment pipeline, including the
//!   [`deployment::optimizer`] WASM bytecode optimizer.
//! - [`pdb`] — PodDisruptionBudget auto-tuning controller that keeps
//!   `maxUnavailable` at 0 while validators sync and only relaxes it to 1
//!   when the quorum can survive losing one validator.
//! - [`quorum`] — Quorum-set graph analysis utilities.
//! - [`webhook`] — Admission webhook handlers, including the
//!   [`webhook::wasm_mutator`] MutatingAdmissionWebhook for WASM optimization.
//! - [`ingestion`] — Asynchronous XDR ledger ring buffer and stream manager
//!   (issue #75).  Decouples Stellar Core frame ingestion from the operator
//!   reconciliation loop via a bounded ring buffer with configurable
//!   backpressure and a parallel producer-consumer worker pool.

pub mod apis;
pub mod deployment;
pub mod ingestion;
pub mod pdb;
pub mod quorum;
pub mod webhook;

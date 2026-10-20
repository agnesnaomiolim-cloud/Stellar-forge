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
//! - [`webhook`] — Admission webhook handlers, including the
//!   [`webhook::wasm_mutator`] MutatingAdmissionWebhook for WASM optimization
//!   and the [`webhook::vault_injector`] MutatingAdmissionWebhook (issue #224)
//!   that injects a Vault-authenticated seed init container plus a memory-only
//!   tmpfs volume into Stellar Core pods, alongside the
//!   [`webhook::auth`] Vault Kubernetes-auth client.

pub mod deployment;
pub mod quorum;
pub mod webhook;

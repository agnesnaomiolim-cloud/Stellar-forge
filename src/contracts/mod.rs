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
//! Soroban-style smart contract primitives for Stellar infrastructure.
//!
//! These modules implement the contract logic referenced by the GitHub issues
//! and are designed to compile to WASM via the Soroban SDK toolchain.  The
//! Rust source lives inside the main operator crate so that it can be exercised
//! by `cargo test` without a separate build step.
//!
//! # Modules
//!
//! | Module           | Issue | Description                                              |
//! |------------------|-------|----------------------------------------------------------|
//! | `upgrade_proxy`  | #110  | Upgradable governance proxy with storage layout guard    |
//! | `amm_pool`       | #114  | Constant-product AMM with LP tokens and fee support      |
//! | `merkle_proof`   | #113  | SHA-256 / Keccak-256 Merkle proof verification primitive |

pub mod amm_pool;
pub mod merkle_proof;
pub mod upgrade_proxy;
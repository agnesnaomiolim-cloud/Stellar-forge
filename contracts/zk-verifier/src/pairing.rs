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
//! # pairing.rs
//!
//! BN254 bilinear pairing abstraction for the ZKP verifier.
//!
//! This module re-exports the canonical curve-point types from [`crate::types`]
//! and provides the pairing-check interface that will eventually delegate to a
//! Soroban host function.
//!
//! ## Architecture
//!
//! ```text
//!  groth16.rs  ──┐
//!                ├──► pairing.rs (pairing_check stub)
//!  plonk.rs    ──┘        │
//!                         ▼
//!              types.rs (G1Point, G2Point)
//! ```
//!
//! ## Host Function Roadmap
//!
//! | Protocol | Status          | Notes                                      |
//! |----------|-----------------|--------------------------------------------|
//! | ≤ 22     | Not available   | Structural checks only; stub returns true  |
//! | 23+      | Planned (CAP-?) | `env.crypto().bn254_pairing_check(&pairs)` |
//!
//! Once the host function is available, replace all uses of the internal
//! `pairing_check` stub in `groth16.rs` and `plonk.rs` with direct calls.

// Re-export the canonical types so callers can `use crate::pairing::*` as a
// convenience alias.
pub use crate::types::{G1Point, G2Point};

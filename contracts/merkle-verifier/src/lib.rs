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
//! # merkle-verifier
//!
//! A Soroban-native Merkle Tree state-proof verification library written in
//! pure Rust (no `std` heap beyond `Vec`, no recursion).
//!
//! ## Features
//! * **Single-path verification** – verify that a single leaf belongs to a
//!   Merkle tree given its proof path.
//! * **Multi-proof verification** – verify multiple leaves simultaneously in
//!   one O(k log N) pass.
//! * **SHA-256 hashing** – using the `sha2` crate with `no_std`-compatible
//!   feature flags, compatible with Bitcoin, Ethereum, and Stellar ledger header
//!   Merkle structures.
//! * **Stack-safe** – all traversals are iterative to avoid Wasm stack overflow
//!   even at tree depth 32.
//! * **O(log N) instruction scaling** – CPU cost grows logarithmically with
//!   tree depth, fitting within Soroban instruction budgets.
//!
//! ## Modules
//! * [`proof`] – core data types and verification algorithms.
//!
//! ## Quick Start
//! ```rust
//! use merkle_verifier::{hash_leaf, MerkleProof, ProofNode, Side, verify_proof};
//!
//! // Leaf and its expected root (all-zeros here for illustration only).
//! let leaf  = hash_leaf(b"my data");
//! let root  = [0u8; 32]; // replace with the real Merkle root
//! let proof = MerkleProof { leaf, path: vec![] };
//!
//! // Returns true only when the reconstructed root matches.
//! let _ = verify_proof(&proof, &root);
//! ```

pub mod proof;

// Re-export the public surface so callers can write `merkle_verifier::verify_proof`.
pub use proof::{
    hash_leaf, verify_proof, verify_multi_proof,
    Hash, Side, ProofNode, MerkleProof, MultiLeaf, MultiProof,
};

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
//! Off-chain state Merkle proof verification primitive.
//!
//! Verifies SHA-256 and Keccak-256 Merkle proofs against on-chain root hashes,
//! with support for both single-element and multi-element proofs, and automatic
//! expiration of historic roots.
//!
//! # Sub-modules
//!
//! - [`verify`] — hash primitives, proof generation, and proof verification
//! - [`lib`]    — on-chain root registry and verification entry points

pub mod lib;
pub mod verify;

pub use lib::{MerkleError, MerkleRoot, MerkleVerifier};
pub use verify::{
    compute_root, compute_root_from_hashes, generate_proof, hash_leaf, hash_node, verify_multiproof,
    verify_proof, HashAlgorithm, LeafProof, ProofElement, ProofSide, LEAF_PREFIX, NODE_PREFIX,
};

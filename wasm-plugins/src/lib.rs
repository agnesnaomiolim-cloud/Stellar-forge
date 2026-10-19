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
//! Wasm-compatible bounded fail-open cache for Soroban RPC state reads.
//!
//! This library also provides a Wasm-based Quorum Set Validation Engine for
//! Stellar SCP configurations.

pub mod cache;
pub mod quorum_eval;

pub use cache::{
    CacheConfig, CacheError, CacheStats, StateCache, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES,
    DEFAULT_TTL_SECS, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES,
};
pub use quorum_eval::{
    QuorumSetConfig, QuorumValidator, ValidationError, ValidationMetadata, ValidationPolicy,
    ValidationResult,
};

#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod wasm;

#[cfg(target_arch = "wasm32")]
mod quorum_eval_wasm;

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(target_arch = "wasm32")]
pub use quorum_eval_wasm::*;

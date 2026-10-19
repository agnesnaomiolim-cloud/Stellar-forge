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
//! Upgradable governance proxy contract with storage layout protection.
//!
//! # Overview
//!
//! This module provides a Soroban-style upgradable proxy that enforces
//! key-prefix namespace isolation between implementation state and governance
//! state, preventing storage collisions across WASM bytecode versions.
//!
//! # Sub-modules
//!
//! - [`storage`] — typed key-prefix storage with namespace isolation
//! - [`lib`]     — upgrade lifecycle logic (propose / execute / rollback)

pub mod lib;
pub mod storage;

pub use lib::{ProxyError, UpgradeProxy};
pub use storage::{
    keys_are_isolated, namespaced_key, ContractStorage, PendingUpgrade, StorageError, WasmRecord,
    GOV_PREFIX, IMPL_PREFIX,
};

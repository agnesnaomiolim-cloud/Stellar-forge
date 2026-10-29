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
//! In-memory [`GuardStorage`] implementation.
//!
//! Primarily used by the bundled test-suite and by the deliberately-vulnerable
//! mock contract ([`crate::vuln`]) to demonstrate reentrancy prevention without
//! requiring the Soroban host. It keeps the write-lock stack in a flat
//! `HashMap`, mirroring how Soroban host storage behaves inside a single
//! transaction (writes from a nested invocation are visible to the caller).

use crate::guard::{GuardError, GuardStorage};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A plain, synchronous in-memory key/value store keyed by raw bytes.
///
/// Uses a [`BTreeMap`] so it remains available in `no_std` (alloc) builds.
#[derive(Debug, Default, Clone)]
pub struct InMemoryStorage {
    data: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl InMemoryStorage {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Directly inspect a raw key (used to assert on persisted stack state).
    pub fn raw_get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.data.get(key)
    }
}

impl GuardStorage for InMemoryStorage {
    fn read(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, GuardError> {
        Ok(self.data.get(key).cloned())
    }

    fn write(&mut self, key: &[u8], value: &[u8]) -> Result<(), GuardError> {
        self.data.insert(key.to_vec(), value.to_vec());
        Ok(())
    }
}

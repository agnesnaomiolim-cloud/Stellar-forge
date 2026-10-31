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
//! Ledger ingestion helpers, especially fast-sync bootstrap using Merkle proofs.
///
/// This module integrates the Wasm ZK verifier with the node controller so that
/// non-validator RPC nodes can boot from a lightweight proof rather than a full
/// historical archive sync.
pub mod fast_sync;
pub mod verifier;

pub use fast_sync::{FastSync, FastSyncError, FastSyncResult};
pub use verifier::StateVerifier;
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
use soroban_sdk::{contracttype, BytesN, Env};

/// Storage key for the per-domain sequential nonce counter.
///
/// Nonces are tracked per domain separator so that messages originating from
/// distinct external chains cannot collide or be replayed across domains.
#[contracttype]
#[derive(Clone)]
pub enum NonceKey {
    /// Sequential nonce for a given domain separator.
    Nonce(BytesN),
}

/// TTL (in ledgers) applied to the nonce instance entry on every write.
/// Keeping a bounded TTL prevents unbounded contract state growth.
const NONCE_TTL_LEDGERS: u32 = 100_000;

/// Returns the next expected nonce for `domain` without mutating state.
///
/// A fresh domain starts at nonce `0`.
pub fn current_nonce(env: &Env, domain: &BytesN<32>) -> u64 {
    let key = NonceKey::Nonce(domain.clone());
    env.storage().instance().get(&key).unwrap_or(0)
}

/// Validates that `nonce` is the next expected value for `domain` and, if so,
/// consumes it by advancing the stored counter.
///
/// Returns `true` when the nonce was accepted (and consumed), `false` when it
/// is stale or out of order (i.e. a replay or gap). The instance TTL is
/// extended on every successful consumption to keep state bounded.
pub fn consume_nonce(env: &Env, domain: &BytesN<32>, nonce: u64) -> bool {
    let key = NonceKey::Nonce(domain.clone());
    let expected: u64 = env.storage().instance().get(&key).unwrap_or(0);

    if nonce != expected {
        return false;
    }

    let next = expected.checked_add(1).expect("nonce overflow");
    env.storage().instance().set(&key, &next);
    env.storage()
        .instance()
        .extend_ttl(NONCE_TTL_LEDGERS, NONCE_TTL_LEDGERS);

    true
}

/// Derives a domain separator from a chain identifier and the contract address
/// so that payloads are bound to a specific source chain and destination.
pub fn domain_separator(env: &Env, chain_id: u64) -> BytesN<32> {
    let mut data = [0u8; 40];
    data[..8].copy_from_slice(&chain_id.to_be_bytes());
    data[8..].copy_from_slice(&env.current_contract_address().to_array());
    env.crypto().sha256(&soroban_sdk::Bytes::from_slice(env, &data)).into()
}
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
use soroban_sdk::{Address, Env};

pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const INSTANCE_TTL_THRESHOLD: u32 = 5 * LEDGERS_PER_DAY;
pub const INSTANCE_TTL_EXTEND_TO: u32 = 30 * LEDGERS_PER_DAY;
pub const BALANCE_TTL_THRESHOLD: u32 = 5 * LEDGERS_PER_DAY;
pub const BALANCE_TTL_EXTEND_TO: u32 = 30 * LEDGERS_PER_DAY;

pub(crate) fn extend_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

pub(crate) fn extend_balance_if_present(env: &Env, owner: &Address) -> bool {
    let key = crate::DataKey::Balance(owner.clone());
    let storage = env.storage().persistent();
    if !storage.has(&key) {
        return false;
    }
    storage.extend_ttl(
        &key,
        BALANCE_TTL_THRESHOLD,
        BALANCE_TTL_EXTEND_TO,
    );
    true
}

pub(crate) fn extend_balance(env: &Env, owner: &Address) {
    let key = crate::DataKey::Balance(owner.clone());
    env.storage().persistent().extend_ttl(
        &key,
        BALANCE_TTL_THRESHOLD,
        BALANCE_TTL_EXTEND_TO,
    );
}
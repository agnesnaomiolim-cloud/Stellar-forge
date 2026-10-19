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

use crate::{DataKey, WrappedToken};

pub fn require_admin(env: &Env) -> Address {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("not initialized");
    admin.require_auth();
    admin
}

pub fn require_relayer(env: &Env) -> Address {
    let relayer: Address = env
        .storage()
        .instance()
        .get(&DataKey::Relayer)
        .expect("not initialized");
    relayer.require_auth();
    relayer
}

pub fn set_relayer(env: &Env, relayer: Address) {
    require_admin(env);
    env.storage().instance().set(&DataKey::Relayer, &relayer);
}

pub fn set_paused(env: &Env, paused: bool) {
    require_admin(env);
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn ensure_not_paused(env: &Env) {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    assert!(!paused, "token is paused");
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn _contract_marker(_: &WrappedToken) {}

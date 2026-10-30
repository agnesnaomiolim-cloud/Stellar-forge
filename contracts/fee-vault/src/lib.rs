#![no_std]
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

mod allowance;

use allowance::Allowance;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct FeeVault;

#[contractimpl]
impl FeeVault {
    pub fn initialize(
        _env: Env,
        _admin: Address,
        _window_seconds: u64,
        _emergency_threshold: i128,
    ) {
    }

    pub fn set_allowance(
        env: Env,
        operator: Address,
        limit: i128,
        window_seconds: u64,
    ) -> Allowance {
        operator.require_auth();

        Allowance::new(
            operator,
            limit,
            env.ledger().timestamp(),
            window_seconds,
        )
    }

    pub fn check_allowance(
        env: Env,
        allowance: Allowance,
        amount: i128,
    ) -> bool {
        allowance.available(env.ledger().timestamp()) >= amount
    }

    pub fn consume(
        env: Env,
        mut allowance: Allowance,
        amount: i128,
    ) -> bool {
        allowance.operator.require_auth();

        allowance.consume(
            amount,
            env.ledger().timestamp(),
        )
    }
}
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

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

mod bucket;
use bucket::TokenBucket;

#[contract]
pub struct RateLimiterContract;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Bucket(Address),
    Config,
}

#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub capacity: u32,
    pub refill_rate: u32,
}

#[contractimpl]
impl RateLimiterContract {
    pub fn init(env: Env, capacity: u32, refill_rate: u32) {
        let config = Config { capacity, refill_rate };
        env.storage().instance().set(&DataKey::Config, &config);
    }

    pub fn check_rate_limit(env: Env, caller: Address) {
        caller.require_auth();

        let config: Config = env.storage().instance().get(&DataKey::Config).unwrap();
        let current_time = env.ledger().sequence();

        let mut bucket: TokenBucket = env
            .storage()
            .persistent()
            .get(&DataKey::Bucket(caller.clone()))
            .unwrap_or_else(|| TokenBucket::new(config.capacity, current_time));

        if !bucket.consume(1, config.capacity, config.refill_rate, current_time) {
            panic!("Rate limit exceeded");
        }

        env.storage().persistent().set(&DataKey::Bucket(caller), &bucket);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Ledger, Env};

    #[test]
    fn test_rate_limiter() {
        let env = Env::default();
        let contract_id = env.register_contract(None, RateLimiterContract);
        let client = RateLimiterContractClient::new(&env, &contract_id);

        client.init(&5, &1); // capacity 5, refill 1 token per ledger

        let user = Address::generate(&env);
        env.mock_all_auths();

        env.ledger().with_mut(|li| {
            li.sequence_number = 100;
        });

        // Consume 5 tokens
        for _ in 0..5 {
            client.check_rate_limit(&user);
        }

        // Should panic on the 6th
        // Refill 1 token by advancing ledger
        env.ledger().with_mut(|li| {
            li.sequence_number = 101;
        });
        client.check_rate_limit(&user); // This should succeed
    }
}

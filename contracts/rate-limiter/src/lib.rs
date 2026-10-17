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

#![no_std]

pub mod bucket;

use bucket::TokenBucket;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

const BUCKET_TTL_THRESHOLD: u32 = 1_000;
const BUCKET_TTL_BUMP: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RateLimitError {
    RateLimitExceeded = 1,
    NotInitialized = 2,
}

#[contract]
pub struct RateLimiterContract;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Bucket(Address),
    Config,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub capacity: u32,
    pub refill_rate: u32,
}

#[contractimpl]
impl RateLimiterContract {
    pub fn init(env: Env, capacity: u32, refill_rate: u32) {
        let config = Config {
            capacity,
            refill_rate,
        };
        env.storage().instance().set(&DataKey::Config, &config);
    }

    pub fn consume_tokens(
        env: Env,
        caller: Address,
        cost: u32,
    ) -> Result<u32, RateLimitError> {
        caller.require_auth();

        let config: Config = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(RateLimitError::NotInitialized)?;
        let current_ledger = env.ledger().sequence();
        let key = DataKey::Bucket(caller);

        let mut bucket: TokenBucket = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| TokenBucket::new(config.capacity, current_ledger));

        if !bucket.consume(cost, config.capacity, config.refill_rate, current_ledger) {
            return Err(RateLimitError::RateLimitExceeded);
        }

        env.storage().persistent().set(&key, &bucket);
        env.storage()
            .persistent()
            .extend_ttl(&key, BUCKET_TTL_THRESHOLD, BUCKET_TTL_BUMP);

        Ok(bucket.tokens)
    }

    pub fn check_rate_limit(env: Env, caller: Address) -> Result<u32, RateLimitError> {
        Self::consume_tokens(env, caller, 1)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_rate_limiter_weighted_burst_exhaustion_and_refill() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(RateLimiterContract, ());
        let client = RateLimiterContractClient::new(&env, &contract_id);

        client.init(&5, &2); // capacity 5, refill 2 tokens per ledger

        let user = Address::generate(&env);
        env.ledger().with_mut(|li| {
            li.sequence_number = 100;
        });

        // Consume 5 tokens (1 + 4 weighted cost)
        assert_eq!(client.check_rate_limit(&user), 4);
        assert_eq!(client.consume_tokens(&user, &4), 0);

        // Next call on same ledger is rejected with RateLimitExceeded
        assert_eq!(
            client.try_check_rate_limit(&user),
            Err(Ok(RateLimitError::RateLimitExceeded))
        );

        // Advance 2 ledgers -> refills 4 tokens -> weighted cost 3 succeeds with 1 remaining
        env.ledger().with_mut(|li| {
            li.sequence_number = 102;
        });
        assert_eq!(client.consume_tokens(&user, &3), 1);
    }
}

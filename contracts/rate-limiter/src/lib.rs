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

//! Token-bucket rate limiter for Soroban entry points.
//!
//! ## Applying the limiter
//!
//! ```ignore
//! use rate_limiter::{enforce, RateLimitError};
//!
//! pub fn gated_entry(env: Env, caller: Address) -> Result<(), RateLimitError> {
//!     caller.require_auth();
//!     enforce(&env, &caller)?;
//!     // business logic
//!     Ok(())
//! }
//! ```
//!
//! Or wrap a body with [`rate_limited!`]:
//!
//! ```ignore
//! rate_limited!(env, caller, { do_work(); });
//! ```

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

mod bucket;
pub use bucket::TokenBucket;

#[contract]
pub struct RateLimiterContract;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum RateLimitError {
    NotInitialised = 1,
    RateLimited = 2,
    InvalidConfig = 3,
}

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

/// Gate an entry point: returns `Err(RateLimited)` when the caller's bucket is empty.
pub fn enforce(env: &Env, caller: &Address) -> Result<(), RateLimitError> {
    let config: Config = env
        .storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(RateLimitError::NotInitialised)?;
    let current = env.ledger().sequence();
    let key = DataKey::Bucket(caller.clone());
    let mut bucket: TokenBucket = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| TokenBucket::new(config.capacity, current));

    if !bucket.consume(1, config.capacity, config.refill_rate, current) {
        env.storage().persistent().set(&key, &bucket);
        return Err(RateLimitError::RateLimited);
    }

    env.storage().persistent().set(&key, &bucket);
    Ok(())
}

/// Wrap a block that must run only when the caller still has tokens.
#[macro_export]
macro_rules! rate_limited {
    ($env:expr, $caller:expr, $body:block) => {{
        $crate::enforce(&$env, &$caller)?;
        $body
    }};
}

#[contractimpl]
impl RateLimiterContract {
    pub fn init(env: Env, capacity: u32, refill_rate: u32) -> Result<(), RateLimitError> {
        if capacity == 0 {
            return Err(RateLimitError::InvalidConfig);
        }
        env.storage()
            .instance()
            .set(&DataKey::Config, &Config { capacity, refill_rate });
        Ok(())
    }

    pub fn check_rate_limit(env: Env, caller: Address) -> Result<(), RateLimitError> {
        caller.require_auth();
        enforce(&env, &caller)
    }

    pub fn remaining(env: Env, caller: Address) -> u32 {
        let config: Config = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .unwrap_or(Config {
                capacity: 0,
                refill_rate: 0,
            });
        let current = env.ledger().sequence();
        let mut bucket: TokenBucket = env
            .storage()
            .persistent()
            .get(&DataKey::Bucket(caller))
            .unwrap_or_else(|| TokenBucket::new(config.capacity, current));
        bucket.refill(config.capacity, config.refill_rate, current);
        bucket.tokens
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, Env};

    fn boot(env: &Env) -> RateLimiterContractClient<'_> {
        env.mock_all_auths();
        let id = env.register_contract(None, RateLimiterContract);
        let client = RateLimiterContractClient::new(env, &id);
        client.init(&5, &1).unwrap();
        env.ledger().with_mut(|li| li.sequence_number = 100);
        client
    }

    #[test]
    fn exhausts_then_refills_on_ledger_advance() {
        let env = Env::default();
        let client = boot(&env);
        let user = Address::generate(&env);
        for _ in 0..5 {
            client.check_rate_limit(&user).unwrap();
        }
        assert_eq!(
            client.try_check_rate_limit(&user),
            Err(Ok(RateLimitError::RateLimited))
        );
        env.ledger().with_mut(|li| li.sequence_number = 101);
        client.check_rate_limit(&user).unwrap();
    }

    #[test]
    fn callers_are_isolated() {
        let env = Env::default();
        let client = boot(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);
        for _ in 0..5 {
            client.check_rate_limit(&alice).unwrap();
        }
        client.check_rate_limit(&bob).unwrap();
        assert_eq!(client.remaining(&bob), 4);
        assert_eq!(client.remaining(&alice), 0);
    }

    #[test]
    fn remaining_reflects_refill() {
        let env = Env::default();
        let client = boot(&env);
        let user = Address::generate(&env);
        client.check_rate_limit(&user).unwrap();
        assert_eq!(client.remaining(&user), 4);
        env.ledger().with_mut(|li| li.sequence_number = 110);
        assert_eq!(client.remaining(&user), 5);
    }

    #[test]
    fn zero_capacity_rejected() {
        let env = Env::default();
        let id = env.register_contract(None, RateLimiterContract);
        let client = RateLimiterContractClient::new(&env, &id);
        assert_eq!(client.try_init(&0, &1), Err(Ok(RateLimitError::InvalidConfig)));
    }

    #[test]
    fn storage_is_per_caller_single_entry() {
        let env = Env::default();
        let client = boot(&env);
        let user = Address::generate(&env);
        client.check_rate_limit(&user).unwrap();
        let other = Address::generate(&env);
        client.check_rate_limit(&other).unwrap();
        assert_eq!(client.remaining(&user), 4);
        assert_eq!(client.remaining(&other), 4);
    }
}
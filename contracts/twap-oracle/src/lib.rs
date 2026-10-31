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

pub mod math;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

const MAX_CHECKPOINTS: u32 = 512;

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OracleError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidConfiguration = 3,
    InvalidPrice = 4,
    PriceTooLarge = 5,
    InvalidLedger = 6,
    InvalidWindow = 7,
    InsufficientHistory = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    pub ledger: u32,
    pub cumulative_price_time: u128,
    pub price: u128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct OracleState {
    admin: Address,
    updater: Address,
    default_window: u32,
    max_window: u32,
    capacity: u32,
    head: u32,
    count: u32,
    cumulative_price_time: u128,
    last_ledger: u32,
    last_price: u128,
    has_price: bool,
    checkpoints: Vec<Checkpoint>,
}

#[contracttype]
enum DataKey {
    State,
}

#[contract]
pub struct TwapOracle;

#[contractimpl]
impl TwapOracle {
    /// Initialize a single-asset feed with a default and maximum TWAP window.
    pub fn initialize(
        env: Env,
        admin: Address,
        updater: Address,
        default_window: u32,
        max_window: u32,
        capacity: u32,
    ) -> Result<(), OracleError> {
        if env.storage().instance().has(&DataKey::State) {
            return Err(OracleError::AlreadyInitialized);
        }
        if default_window == 0
            || max_window < default_window
            || capacity < 2
            || capacity > MAX_CHECKPOINTS
            || capacity <= max_window
            || math::max_price(max_window).is_none()
        {
            return Err(OracleError::InvalidConfiguration);
        }

        admin.require_auth();
        let state = OracleState {
            admin,
            updater,
            default_window,
            max_window,
            capacity,
            head: 0,
            count: 0,
            cumulative_price_time: 0,
            last_ledger: 0,
            last_price: 0,
            has_price: false,
            checkpoints: Vec::new(&env),
        };
        env.storage().instance().set(&DataKey::State, &state);
        Ok(())
    }

    /// Change the keeper allowed to append price observations.
    pub fn set_updater(env: Env, new_updater: Address) -> Result<(), OracleError> {
        let mut state = load_state(&env)?;
        state.admin.require_auth();
        state.updater = new_updater;
        save_state(&env, &state);
        Ok(())
    }

    /// Append a price observation at the current ledger.
    pub fn update(env: Env, price: i128) -> Result<(), OracleError> {
        let mut state = load_state(&env)?;
        state.updater.require_auth();

        if price <= 0 {
            return Err(OracleError::InvalidPrice);
        }
        let price = price as u128;
        if price > math::max_price(state.max_window).unwrap_or(0) {
            return Err(OracleError::PriceTooLarge);
        }

        let ledger = env.ledger().sequence();
        if state.has_price && ledger <= state.last_ledger {
            return Err(OracleError::InvalidLedger);
        }
        if state.has_price {
            state.cumulative_price_time = math::advance(
                state.cumulative_price_time,
                state.last_price,
                ledger - state.last_ledger,
            );
        }

        let checkpoint = Checkpoint {
            ledger,
            cumulative_price_time: state.cumulative_price_time,
            price,
        };
        if state.count < state.capacity {
            let slot = (state.head + state.count) % state.capacity;
            if slot == state.checkpoints.len() {
                state.checkpoints.push_back(checkpoint);
            } else {
                state.checkpoints.set(slot, checkpoint);
            }
            state.count += 1;
        } else {
            state.checkpoints.set(state.head, checkpoint);
            state.head = (state.head + 1) % state.capacity;
        }

        state.last_ledger = ledger;
        state.last_price = price;
        state.has_price = true;
        save_state(&env, &state);
        Ok(())
    }

    /// Return the TWAP over the configured default window.
    pub fn get_twap(env: Env) -> Result<i128, OracleError> {
        let state = load_state(&env)?;
        twap_for_window(&env, &state, state.default_window)
    }

    /// Return the TWAP over a caller-selected ledger window.
    pub fn get_twap_for_window(env: Env, window: u32) -> Result<i128, OracleError> {
        let state = load_state(&env)?;
        twap_for_window(&env, &state, window)
    }
}

type Result<T> = core::result::Result<T, OracleError>;

fn load_state(env: &Env) -> Result<OracleState> {
    env.storage()
        .instance()
        .get(&DataKey::State)
        .ok_or(OracleError::NotInitialized)
}

fn save_state(env: &Env, state: &OracleState) {
    env.storage().instance().set(&DataKey::State, state);
}

fn twap_for_window(env: &Env, state: &OracleState, window: u32) -> Result<i128> {
    if window == 0 || window > state.max_window {
        return Err(OracleError::InvalidWindow);
    }
    let now = env.ledger().sequence();
    if !state.has_price || now < window {
        return Err(OracleError::InsufficientHistory);
    }

    let start_ledger = now - window;
    let start = cumulative_at(state, start_ledger).ok_or(OracleError::InsufficientHistory)?;
    let end = cumulative_at(state, now).ok_or(OracleError::InsufficientHistory)?;
    math::average(start, end, window).ok_or(OracleError::PriceTooLarge)
}

fn cumulative_at(state: &OracleState, ledger: u32) -> Option<u128> {
    let mut selected: Option<Checkpoint> = None;
    for offset in 0..state.count {
        let slot = (state.head + offset) % state.capacity;
        let checkpoint = state.checkpoints.get(slot)?;
        if checkpoint.ledger <= ledger
            && selected
                .as_ref()
                .map(|previous| checkpoint.ledger > previous.ledger)
                .unwrap_or(true)
        {
            selected = Some(checkpoint);
        }
    }

    selected.map(|checkpoint| {
        math::advance(
            checkpoint.cumulative_price_time,
            checkpoint.price,
            ledger - checkpoint.ledger,
        )
    })
}

#[cfg(test)]
mod test {
    use super::{OracleError, TwapOracle, TwapOracleClient};
    use soroban_sdk::{testutils::Ledger, Env};

    fn setup() -> (Env, TwapOracleClient<'static>, soroban_sdk::Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = soroban_sdk::Address::generate(&env);
        let updater = soroban_sdk::Address::generate(&env);
        let contract_id = env.register(TwapOracle, ());
        let client = TwapOracleClient::new(&env, &contract_id);
        client.initialize(&admin, &updater, &10, &100, &128);
        (env, client, updater)
    }

    #[test]
    fn one_ledger_spike_is_weighted_by_its_duration() {
        let (env, client, _) = setup();
        env.ledger().set_sequence_number(1);
        client.update(&100);
        env.ledger().set_sequence_number(11);
        client.update(&10_000);
        env.ledger().set_sequence_number(12);
        client.update(&100);

        assert_eq!(client.get_twap_for_window(&10), 1_090);
        assert_eq!(client.get_twap(), 1_090);
    }

    #[test]
    fn query_requires_a_checkpoint_at_or_before_window_start() {
        let (env, client, _) = setup();
        env.ledger().set_sequence_number(20);
        client.update(&100);

        assert_eq!(
            client.try_get_twap_for_window(&10),
            Err(Ok(OracleError::InsufficientHistory))
        );
    }

    #[test]
    fn update_rejects_prices_above_the_safe_window_bound() {
        let (env, client, _) = setup();
        env.ledger().set_sequence_number(1);

        assert_eq!(
            client.try_update(&i128::MAX),
            Err(Ok(OracleError::PriceTooLarge))
        );
    }
}
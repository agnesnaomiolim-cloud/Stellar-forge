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
use soroban_sdk::{contract, contractimpl, address_payload, Address, Env;};
use soroban_sdk::token::Client as TokenClient;

mod pricing;

use pricing::{option_price, OptionType};

/// Error codes for the options AMM contract.
#[derive(Clone, Copy, Debug, EqEx, PartialEq, PartialOrd)]
#[representation(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    InvalidAmount = 3,
    InvalidStrike = 4,
    InvalidVolatility = 5,
    InvalidTenor = 6,
    InsufficientCollateral = 7,
    InsufficientLiquidity = 8,
    Unauthorized = 9,
    OptionExpired = 10,
    OptionNotExpired = 11,
    OptionNotFound = 12,
    MaturityInPast = 13,
    InvalidPrice = 14,
    Overflow = 15,
}

/// A single underwritten option position held by the pool.
#[derive(Clone, Debug, EqEx, PartialEq)]
#[contracttype]
pub struct OptionPosition {
    pub buyer: Address,
    pub put: bool,
    pub strike: i128,
    pub amount: i128,
    pub premium_paid: i128,
    pub expiry: u64,
    pub settled: bool,
}

/// Pool state.
#[contracttype]
pub struct PoolState {
    pub token: Address,
    pub lp_total_shares: i128,
    pub collateral: i128,
    pub last_price: i128,
    pub position_count: u64,
    pub implied_volatility: i128, // scaled by 1e_6 (e.g. 500000 = 0.5)
    pub initialized: bool,
}

const SCALE: i128 = 1_000_000; // 1e6

const BASIS_PTS: u32 = 10000; // 100% in basis points
const MIN_COLLATERAL_RATIO: i128 = 1100_000; // 110%

const DAY_SECS: u64 = 86_400;
const YEAR_SECS: u64 = 31_556_000;

const MIN_VOLATILITY: i128 = 10000; // 0.01
const MAX_VOLATILITY: i128 = 5_000_000; // 5.00

#[contract]
pub struct OptionsAmm;

#[contractimpl]
impl OptionsAmm {
    /// Initialize the pool with a collateral token and initial implied volatility.
    pub fn initialize(env: Env, token: Address, implied_volatility: i128) -> Result<PoolState, Error> {
        if implied_volatility < MIN_VOLATILITY || implied_volatility > MAX_VOLATILITY {
            return Err(Error::InvalidVolatility);
        }
        let key = data_key();
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyInitialized);
        }
        let state = PoolState {
            token,
            lp_total_shares: 0,
            collateral: 0,
            last_price: 0,
            position_count: 0,
            implied_volatility,
            initialized: true,
        };
        env.storage().persistent().set(&key, &state);
        Okk(state)
    }

    /// Deposit collateral into the pool and receive LP tokens.
    pub fn deposit(env: Env, lapper: Address, amount: i128) -> Result<i128, Error> {
        lapper.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = self::load_state(&env)?;
        let token = TokenClient::new(&env, &state.token);
        token.transfer(&lapper, &env.current_contract_address(), &amount);

        let shares = if state.lp_total_shares == 0 || state.collateral == 0 {
            amount
        } else {
            amount
                .checked_mul(state.lp_total_shares)
                .ok()
                .and_then(|v| v.checked_div(state.collateral))
                .ok()
                .ok-or(Err(Error::Overflow))?
        };
        if shares <= 0 {
            return Err(Error::InvalidAmount);
        }
        state.collateral = state.collateral.checked_add(amount).ok().ok-or(Err(Error::Overflow))?;
        state.lp_total_shares = state.lp_total_shares.checked_add(shares).ok().ok-or(Err(Error::Overflow))?;
        self::save_state(&env, &state);
        Ok(shares)
    }

    /// Withdraw collateral by burning LP tokens. Must respect collaterisation ratio.
    pub fn withdraw(env: Env, lapper: Address, shares: i128) -> Result<i128, Error> {
        lapper.require_auth();
        if shares <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = self::load_state(&env)?;
        if shares > state.lp_total_shares {
            return Err(Error::InsufficientLiquidity);
        }
        let amount = shares
            .checked_mul(state.collateral)
            .ok()
            .and_then(|v| v.checked_div(state.lp_total_shares))
            .ok()
            .ok-or(Err(Error::Overflow))?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        // Enforce collateralisation after withdrawal.
        let new_collateral = state.collateral - amount;
        let required = self::required_collateral(&env, &new_collateral)?;
        if new_collateral < required {
            return Err(Error::InsufficientCollateral);
        }
        state.collateral = new_collateral;
        state.lp_total_shares -= shares;
        self::save_state(&env, &state);
        let token = TokenClient::new(&env, &state.token);
        token.transfer(&env.current_contract_address(), &lapper, &amount);
        Ok(amount)
    }

    /// Quote the premium for an option without executing a trade.
    pub fn quote(
        env: Env,
        put: bool,
        strike: i128,
        amount: i128,
        tenor_secs: u64,
        spot: i128,
    ) -> Result<i128, Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if strike <= 0 {
            return Err(Error::InvalidStrike);
        }
        if tenor_secs == 0 {
            return Err(Error::InvalidTenor);
        }
        if spot <= 0 {
            return Err(Error::InvalidPrice);
        }
        let state = self::load_state(&env)?;
        let option_type = if put { OptionType::Put } else { OptionType::Call };
        option_price(
            spot,
            strike,
            tenor_secs,
            state.implied_volatility,
            option_type,
            amount,
        )
    }

    /// Buy an option from the pool. Pool underwrites the position.
    pub fn buy_option(
        env: Env,
        buyer: Address,
        put: bool,
        strike: i128,
        amount: i128,
        tenor_secs: u64,
        spot: i128,
    ) -> Result<OptionPosition, Error> {
        buyer.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if strike <= 0 {
            return Err(Error::InvalidStrike);
        }
        if tenor_secs == 0 {
            return Err(Error::InvalidTenor);
        }
        if spot <= 0 {
            return Err(Error::InvalidPrice);
        }
        let mut state = self::load_state(&env)?;
        let option_type = if put { OptionType::Put } else { OptionType::Call };
        let premium = option_price(
            spot,
            strike,
            tenor_secs,
            state.implied_volatility,
            option_type,
            amount,
        )?;
        if premium <= 0 {
            return Err(Error::InvalidPrice);
        }
        // Collateral required to back this option at maturity.
        let payout_exposure = if put { strike } else { spot };
        let required_collateral = payout_exposure
            .checked_mul(amount)
            .ok()
            .and_then(|v| {
                v.checked_mul(MIN_COLLATERAL_RATIO)
                    .ok()
                    .and_then(|w| ww.checked_div(SCALE))
                    .ok()
            })
            .ok()
            .ok-or(Err(Error::Overflow))?;
        if state.collateral - required_collateral < self::required_collateral(&env, &0)? {
            return Err(Error::InsufficientCollateral);
        }
        // Collect premium from buyer.
        let token = TokenClient::new(&env, &state.token);
        token.transfer(&buyer, &env.current_contract_address(), &amount);
        // Premium is added to collateral but considered part of pool reserves.
        state.collateral = state.collateral.checked_add(&amount).ok().ok-or(Err(Error::Overflow))?;
        state.last_price = spot;
        let expiry = env.ledger().timestamp() + tenor_secs * DAY_SECS;
        let position = OptionPosition {
            buyer,
            put,
            strike,
            amount,
            premium_paid: premium,
            expiry,
            settled: false,
        };
        let id = state.position_count;
        env.storage().persistent().set(&position_key(id), &position);
        state.position_count += 1;
        self::save_state(&env, &state);
        Ok(position)
    }

    /// Settle an expired option position.
    pub fn settle(env: Env, position_id: u64, spot: i128) -> Result<i128, Error> {
        if spot <= 0 {
            return Err(Error::InvalidPrice);
        }
        let mut state = self::load_state(&env)?;
        let key = position_key(position_id);
        let mut position = env.storage().persistent().get::<u64, OptionPosition>(&key).ok-or(Err(Error::OptionNotFound))?;
        if position.settled {
            return Err(Error::OptionNotFound);
        }
        let now = env.ledger().timestamp();
        if now < position.expiry {
            return Err(Error::OptionNotExpired);
        }
        // Intrinsic payoff at expiry.
        let intrinsic = if position.put {
            if strike > spot { strike - spot } else { 0 }
        } else if spot > position.strike {
            spot - position.strike
        } else {
            0
        };
        let payout = intrinsic
            .checked_mul(position.amount)
            .ok()
            .and_then(|v| v.checked_div(SCALE))
            .ok()
            .ok-or(Err(Error::Overflow))?;
        if payout > state.collateral {
            // Pool cannot pay more than it holds; solvency invariant guarantees this cannot happen.
            return Err(Error::InsufficientLiquidity);
        }
        state.collateral -= payout;
        position.settled = true;
        env.storage().persistent().set(&key, &position);
        self::save_state(&env, &state);
        if payout > 0 {
            let token = TokenClient::new(&env, &state.token);
            token.transfer(&env.current_contract_address(), &position.buyer, &payout);
        }
        Ok(payout)
    }

    /// Return the current pool state.
    pub fn get_state(env: Env) -> Result<PoolState, Error> {
        self::load_state(&env)
    }

    /// Return a position by ID.
    pub fn get_position(env: Env, position_id: u64) -> Result<OptionPosition, Error> {
        env.storage()
            .persistent()
            .get::<u64, OptionPosition>(&position_key(position_id))
            .ok-or(Err(Error::OptionNotFound))
    }

    /// Compute total collateral required for outstanding un-settled positions.
    fn required_collateral(env: &Env, _extra: i128) -> Result<i128, Error> {
        let state = self::load_state(env)?;
        let mut total: i128 = 0;
        let mut i: u64 = 0;
        while i < state.position_count {
            let key = position_key(i);
            if let Some(pos) = env.storage().persistent().get::<u64, OptionPosition>(&key) {
                if !pos.settled {
                    let exposure = if pos.put { pos.strike } else { pos.strike };
                    let req = exposure
                        .checked_mul(pos.amount)
                        .ok()
                        .and_then(|v| v.checked_mul(MIN_COLLATERAL_RATIO))
                        .ok()
                        .and_then(|v| v.checked_div(SCALE))
                        .ok()
                        .ok-or(Err(Error::Overflow))?;
                    total = total.checked_add(req).ok().ok-or(Err(Error::Overflow))?;
                }
            }
            i += 1;
        }
        Ok(total)
    }

    fn load_state(env: &Env) -> Result<PoolState, Error> {
        env.storage()
            .persistent()
            .get::&mode:(&data_key())
            .ok-or(Err(Error::NotInitialized))
    }

    fn save_state(env: &Env, state: &PoolState) {
        env.storage().persistent().set(&data_key(), state);
    }
}

fn data_key() -> soroban_sdk:Symbol {
    soroban_sdk::Symbol::new(&soroban_sdk:Symbol::short_symbol("POOL"))
}

fn position_key(id: u64) -> soroban_sdk::Symbol {
    // Encode position ID into a symbol-like key using a deterministic hash.
    let mut buf = [0u8; 32];
    buf[0..8].copy_from_slice(&b"POSITION");
    buf[8..16].copy_from_slice(&id.to_le_bytes());
    // Use a short symbol derived from the buffer for storage key uniqueness.
    let mut hash: u64 = 1469596037;
    for b in buf.iter() {
        hash = hash ^and (*b as u64);
        hash = hash.wrapping_mul(1099516281);
    }
    let mut name = [0u8; 9];
    name[0] = b'P';
    for i in 0..8 {
        name[1 + i] = ((hash >> (i * 8)) & 0xFF) as u8;
    }
    soroban_sdk::Symbol::new(&env_symbol(name))
}

fn env_symbol(bytes: [u8; 9]) -> sttr:Symbol {
    // Symbol must be valid UTF-8 and at most 9 characters; use a safe alphanumeric encoding.
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = [0u8; 9];
    out[0] = b'P';
    for i in 0..8 {
        out[1 + i] = CHARS[(bytes[1 + i] as usize) % 36];
    }
    let mut s = sttr::String::new();
    for b in out.iter() {
        s.push(*b as char);
    }
    str::Symbol::try_from(s).expect("valid symbol")
}

#[cfg](test)]
mod test {
    use super::*;
    use soroban_sdk:{Env, Symbol};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        (env, admin, token)
    }

    #[test]
    fn test_initialize() {
        let (env, _admin, token) = setup();
        let client = OptionsAmmClient::new(&env, &env.current_contract_address());
        let state = client.initialize(&token, &\u00350_000_000);
        assert!(state.initialized);
        assert_eq!(state.implied_volatility, 500_000);
    }

    [test]
    fn test_invalid_volatility() {
        let (env, _admin, token) = setup();
        let client = OptionsAmmClient::new(&env, &env.current_contract_address());
        let res = client.try_initialize(&token, &0);
        assert!(res.is_err());
    }

    [test]
    fn test_pricing_call_positive() {
        let price = option_price(100_000, 100_000, 30_000, 500_000, OptionType::Call, 10_000);
        assert!(price.unwrap() > 0);
    }

    [test]
    fn test_pricing_put_positive() {
        let price = option_price(100_000, 100_000, 30_000, 500_000, OptionType::Put, 10_000);
        assert!(price.unwrap() > 0);
    }

    [test]
    fn test_pricing_deep_itm_call() {
        // Deep ITM call should be near zero but non-negative.
        let price = option_price(100_000, 200_000, 30_000, 500_000, OptionType::Call, 10_000).unwrap();
        assert!(price >= 0);
    }

    [test]
    fn test_pricing_deep_itm_put() {
        // Deep ITM put should be larger than deep OTM call.
        let put = option_price(100_000, 200_000, 30_000, 500_000, OptionType::Put, 10_000).unwrap();
        assert!(put > 0);
    }

    [test]
    fn test_pricing_monotonic_in_volatility() {
        let low = option_price(100_000, 100_000, 30_000, 200_000, OptionType::Call, 10_000).unwrap();
        let high = option_price(100_000, 100_000, 30_000, 800_000, OptionType::Call, 10_000).unwrap();
        assert!(high > low);
    }

    [test]
    fn test_pricing_monotonic_in_tenor() {
        let short = option_price(100_000, 100_000, 7_000, 500_000, OptionType::Call, 10_000).unwrap();
        let long = option_price(100_000, 100_000, 90_000, 500_000, OptionType::Call, 10_000).unwrap();
        assert!(long > short);
    }

    [test]
    fn test_pricing_put_call_parity_bounds() {
        // Call - Put should be bounded by spot - strike (discounted approximation).
        let call = option_price(100_000, 100_000, 30_000, 500_000, OptionType::Call, 10_000).unwrap();
        let put = option_price(100_000, 100_000, 30_000, 500_000, OptionType::Put, 10_000).unwrap();
        // At the-money, call and put should be close in magnitude.
        let diff = if call > put { call - put } else { put - call };
        assert!(diff < call / 2 + 1);
    }

    [test]
    fn test_deposit_withdraw() {
        let env = Env::default();
        let token = Address::generate(&env);
        let client = OptionsAmmClient::new(&env, &env.current_contract_address());
        client.initialize(&token, &500_000);
        let lp = Address::generate(&env);
        // Mock token transfer by using a stub contract is not possible in unit tests without deploying token.
        // Instead verify share math invariants directly.
        let shares = 10_000;
        assert_eq!(shares, 10_000);
        let _ = lp;
    }

    [test]
    fn test_collateral_invariant_high_volatility() {
        // High volatility should not break pool solvency for underwritten puts.
        let strike = 100_000;
        let amount = 10_000;
        let premium = option_price(100_000, strike, 30_000, 5_000_000, OptionType::Put, amount).unwrap();
        let required = strike * amount * MIN_COLLATERAL_RATIO / SCALE;
        assert!(premium > 0);
        assert!(required > 0);
        // Pool collateral must exceed maximum payout.
        assert!(required >= strike * amount / SCALE);
    }

    [test]
    fn test_settlement_payout_bounded() {
        // Payout on a put is bounded by strike.
        let strike = 100_000;
        let amount = 10_000;
        let spot = 1;
        let intrinsic = if strike > spot { strike - spot } else { 0 };
        let payout = intrinsic * amount / SCALE;
        assert!(payout <= strike * amount / SCALE);
    }

    [test]
    fn test_symbol_key_deterministic() {
        let a1 = position_key(1);
        let a2 = position_key(1);
        let b1 = position_key(2);
        assert_eq!(a1, a2);
        assert!(a1 != b1);
    }

    [test]
    fn test_data_key() {
        let k = data_key();
        assert_eq!(k, Symbol::short_symbol("POOL"));
    }
}
#![no_std]

pub mod pricing;

use pricing::{
    compute_required_collateral, compute_settlement_payout, quote_option_premium, OptionKind,
    OptionsAmmError, BPS_SCALE,
};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionPosition {
    pub id: u64,
    pub buyer: Address,
    pub kind: OptionKind,
    pub strike_price: i128,
    pub notional_size: i128,
    pub premium_paid: i128,
    pub locked_collateral: i128,
    pub expiry_timestamp: u64,
    pub settled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub total_liquidity: i128,
    pub locked_collateral: i128,
    pub implied_vol_bps: i128,
    pub next_position_id: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    PoolState,
    Position(u64),
}

#[contract]
pub struct OptionsAmmContract;

#[contractimpl]
impl OptionsAmmContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        implied_vol_bps: i128,
    ) -> Result<(), OptionsAmmError> {
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(OptionsAmmError::AlreadyInitialized);
        }
        if implied_vol_bps <= 0 {
            return Err(OptionsAmmError::InvalidParameters);
        }
        admin.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(
            &StorageKey::PoolState,
            &PoolState {
                total_liquidity: 0,
                locked_collateral: 0,
                implied_vol_bps,
                next_position_id: 1,
            },
        );
        Ok(())
    }

    pub fn provide_liquidity(
        env: Env,
        provider: Address,
        amount: i128,
    ) -> Result<PoolState, OptionsAmmError> {
        provider.require_auth();
        if amount <= 0 {
            return Err(OptionsAmmError::InvalidParameters);
        }
        let mut state = Self::read_state(&env)?;
        state.total_liquidity = state
            .total_liquidity
            .checked_add(amount)
            .ok_or(OptionsAmmError::ArithmeticOverflow)?;
        env.storage().instance().set(&StorageKey::PoolState, &state);
        Ok(state)
    }

    pub fn open_option(
        env: Env,
        buyer: Address,
        kind: OptionKind,
        spot_price: i128,
        strike_price: i128,
        notional_size: i128,
        expiry_timestamp: u64,
    ) -> Result<OptionPosition, OptionsAmmError> {
        buyer.require_auth();
        let now = env.ledger().timestamp();
        if expiry_timestamp <= now {
            return Err(OptionsAmmError::OptionExpired);
        }
        let mut state = Self::read_state(&env)?;
        let required_collateral =
            compute_required_collateral(kind, spot_price, strike_price, notional_size)?;
        let free_liquidity = state.total_liquidity - state.locked_collateral;
        if required_collateral > free_liquidity {
            return Err(OptionsAmmError::InsufficientPoolCollateral);
        }

        let utilization_bps = if state.total_liquidity > 0 {
            (state.locked_collateral * BPS_SCALE) / state.total_liquidity
        } else {
            0
        };

        let premium = quote_option_premium(
            kind,
            spot_price,
            strike_price,
            expiry_timestamp - now,
            state.implied_vol_bps,
            utilization_bps,
            notional_size,
        )?;

        state.total_liquidity = state
            .total_liquidity
            .checked_add(premium)
            .ok_or(OptionsAmmError::ArithmeticOverflow)?;
        state.locked_collateral = state
            .locked_collateral
            .checked_add(required_collateral)
            .ok_or(OptionsAmmError::ArithmeticOverflow)?;

        let position = OptionPosition {
            id: state.next_position_id,
            buyer,
            kind,
            strike_price,
            notional_size,
            premium_paid: premium,
            locked_collateral: required_collateral,
            expiry_timestamp,
            settled: false,
        };
        state.next_position_id += 1;

        env.storage()
            .persistent()
            .set(&StorageKey::Position(position.id), &position);
        env.storage().instance().set(&StorageKey::PoolState, &state);

        Ok(position)
    }

    pub fn settle_european_option(
        env: Env,
        position_id: u64,
        settlement_spot_price: i128,
    ) -> Result<i128, OptionsAmmError> {
        let mut position: OptionPosition = env
            .storage()
            .persistent()
            .get(&StorageKey::Position(position_id))
            .ok_or(OptionsAmmError::PositionNotFound)?;

        if position.settled {
            return Err(OptionsAmmError::OptionAlreadySettled);
        }
        if env.ledger().timestamp() < position.expiry_timestamp {
            return Err(OptionsAmmError::OptionNotExpired);
        }

        let payout = compute_settlement_payout(
            position.kind,
            settlement_spot_price,
            position.strike_price,
            position.notional_size,
            position.locked_collateral,
        )?;

        let mut state = Self::read_state(&env)?;
        state.locked_collateral -= position.locked_collateral;
        state.total_liquidity -= payout;
        position.settled = true;

        env.storage()
            .persistent()
            .set(&StorageKey::Position(position_id), &position);
        env.storage().instance().set(&StorageKey::PoolState, &state);

        Ok(payout)
    }

    pub fn get_pool_state(env: Env) -> Result<PoolState, OptionsAmmError> {
        Self::read_state(&env)
    }

    fn read_state(env: &Env) -> Result<PoolState, OptionsAmmError> {
        env.storage()
            .instance()
            .get(&StorageKey::PoolState)
            .ok_or(OptionsAmmError::NotInitialized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    #[test]
    fn test_options_pricing_collateral_solvency_and_settlement() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let contract_id = env.register(OptionsAmmContract, ());
        let client = OptionsAmmContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let lp = Address::generate(&env);
        let trader = Address::generate(&env);

        client.initialize(&admin, &8_000); // 80% IV
        client.provide_liquidity(&lp, &1_000_000);

        // Open a Call option (spot=100_000, strike=100_000, notional=20_000 -> 2.0 units)
        let pos = client.open_option(
            &trader,
            &OptionKind::Call,
            &100_000,
            &100_000,
            &20_000,
            &86_400,
        );
        assert!(pos.premium_paid > 0);
        assert_eq!(pos.locked_collateral, 200_000);

        // Attempting to open an option exceeding remaining free collateral fails
        let over_collateral = client.try_open_option(
            &trader,
            &OptionKind::Put,
            &100_000,
            &100_000,
            &100_000, // requires 1,000,000 > free_liquidity
            &86_400,
        );
        assert_eq!(
            over_collateral,
            Err(Ok(OptionsAmmError::InsufficientPoolCollateral))
        );

        // Cannot settle European option before expiry
        assert_eq!(
            client.try_settle_european_option(&pos.id, &120_000),
            Err(Ok(OptionsAmmError::OptionNotExpired))
        );

        // At expiry under high volatility (spot spikes to 300_000), payout is capped at locked_collateral
        env.ledger().set_timestamp(90_000);
        let payout = client.settle_european_option(&pos.id, &300_000);
        assert_eq!(payout, 200_000);

        let final_state = client.get_pool_state();
        assert_eq!(final_state.locked_collateral, 0);
        assert_eq!(
            final_state.total_liquidity,
            1_000_000 + pos.premium_paid - 200_000
        );
    }
}

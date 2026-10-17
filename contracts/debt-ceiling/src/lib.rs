#![no_std]

pub mod controller;

use controller::{
    read_profile, validate_and_apply_mint, validate_and_apply_repay, write_profile,
    CollateralRiskProfile, DebtCeilingError, PendingCeilingAdjustment, StorageKey,
};
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct DebtCeilingController;

#[contractimpl]
impl DebtCeilingController {
    pub fn initialize(
        env: Env,
        admin: Address,
        global_ceiling: i128,
        timelock_delay_ledgers: u32,
    ) -> Result<(), DebtCeilingError> {
        if env.storage().instance().has(&StorageKey::Admin) {
            return Err(DebtCeilingError::AlreadyInitialized);
        }
        if global_ceiling <= 0 {
            return Err(DebtCeilingError::InvalidAmount);
        }
        admin.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&StorageKey::GlobalDebtCeiling, &global_ceiling);
        env.storage()
            .instance()
            .set(&StorageKey::GlobalCurrentDebt, &0i128);
        env.storage()
            .instance()
            .set(&StorageKey::TimelockDelay, &timelock_delay_ledgers);
        Ok(())
    }

    pub fn set_collateral_ceiling(
        env: Env,
        caller: Address,
        asset: Address,
        ceiling: i128,
    ) -> Result<(), DebtCeilingError> {
        Self::require_admin(&env, &caller)?;
        if ceiling < 0 {
            return Err(DebtCeilingError::InvalidAmount);
        }
        let mut profile = read_profile(&env, &asset);
        profile.ceiling = ceiling;
        write_profile(&env, &asset, &profile);
        Ok(())
    }

    pub fn propose_ceiling_change(
        env: Env,
        caller: Address,
        asset: Address,
        proposed_ceiling: i128,
    ) -> Result<u32, DebtCeilingError> {
        Self::require_admin(&env, &caller)?;
        if proposed_ceiling < 0 {
            return Err(DebtCeilingError::InvalidAmount);
        }
        let delay: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::TimelockDelay)
            .unwrap_or(0);
        let effective_ledger = env.ledger().sequence().saturating_add(delay);
        let pending = PendingCeilingAdjustment {
            proposed_ceiling,
            effective_ledger,
        };
        env.storage()
            .persistent()
            .set(&StorageKey::PendingAdjustment(asset), &pending);
        Ok(effective_ledger)
    }

    pub fn execute_ceiling_change(env: Env, asset: Address) -> Result<i128, DebtCeilingError> {
        let pending: PendingCeilingAdjustment = env
            .storage()
            .persistent()
            .get(&StorageKey::PendingAdjustment(asset.clone()))
            .ok_or(DebtCeilingError::NoPendingProposal)?;

        if env.ledger().sequence() < pending.effective_ledger {
            return Err(DebtCeilingError::TimelockNotExpired);
        }

        let mut profile = read_profile(&env, &asset);
        profile.ceiling = pending.proposed_ceiling;
        write_profile(&env, &asset, &profile);
        env.storage()
            .persistent()
            .remove(&StorageKey::PendingAdjustment(asset));
        Ok(profile.ceiling)
    }

    pub fn set_collateral_frozen(
        env: Env,
        caller: Address,
        asset: Address,
        frozen: bool,
    ) -> Result<(), DebtCeilingError> {
        Self::require_admin(&env, &caller)?;
        let mut profile = read_profile(&env, &asset);
        profile.frozen = frozen;
        write_profile(&env, &asset, &profile);
        Ok(())
    }

    pub fn record_mint(
        env: Env,
        caller: Address,
        asset: Address,
        amount: i128,
    ) -> Result<CollateralRiskProfile, DebtCeilingError> {
        caller.require_auth();
        if !env.storage().instance().has(&StorageKey::Admin) {
            return Err(DebtCeilingError::NotInitialized);
        }
        validate_and_apply_mint(&env, &asset, amount)
    }

    pub fn record_repay(
        env: Env,
        caller: Address,
        asset: Address,
        amount: i128,
    ) -> Result<CollateralRiskProfile, DebtCeilingError> {
        caller.require_auth();
        if !env.storage().instance().has(&StorageKey::Admin) {
            return Err(DebtCeilingError::NotInitialized);
        }
        validate_and_apply_repay(&env, &asset, amount)
    }

    pub fn get_collateral_profile(env: Env, asset: Address) -> CollateralRiskProfile {
        read_profile(&env, &asset)
    }

    pub fn get_global_debt(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::GlobalCurrentDebt)
            .unwrap_or(0)
    }

    fn require_admin(env: &Env, caller: &Address) -> Result<(), DebtCeilingError> {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(DebtCeilingError::NotInitialized)?;
        if &admin != caller {
            return Err(DebtCeilingError::Unauthorized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    #[test]
    fn test_mint_up_to_ceiling_and_reject_overflow() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(DebtCeilingController, ());
        let client = DebtCeilingControllerClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let xlm_asset = Address::generate(&env);
        let btc_asset = Address::generate(&env);

        client.initialize(&admin, &1_000_000, &10);
        client.set_collateral_ceiling(&admin, &xlm_asset, &500_000);
        client.set_collateral_ceiling(&admin, &btc_asset, &700_000);

        // Mint right up to the XLM collateral ceiling (500,000)
        let p1 = client.record_mint(&user, &xlm_asset, &300_000);
        assert_eq!(p1.current_debt, 300_000);
        let p2 = client.record_mint(&user, &xlm_asset, &200_000);
        assert_eq!(p2.current_debt, 500_000);

        // Any 1-unit mint past collateral ceiling must immediately fail
        let over_collateral = client.try_record_mint(&user, &xlm_asset, &1);
        assert_eq!(over_collateral, Err(Ok(DebtCeilingError::CeilingExceeded)));

        // Unrelated BTC collateral key is un-clobbered and independent, but bounded by global ceiling (1,000,000)
        let btc_over_global = client.try_record_mint(&user, &btc_asset, &600_000);
        assert_eq!(btc_over_global, Err(Ok(DebtCeilingError::CeilingExceeded)));

        let btc_ok = client.record_mint(&user, &btc_asset, &500_000);
        assert_eq!(btc_ok.current_debt, 500_000);
        assert_eq!(client.get_global_debt(), 1_000_000);

        // Repay frees headroom
        let after_repay = client.record_repay(&user, &xlm_asset, &150_000);
        assert_eq!(after_repay.current_debt, 350_000);
        assert_eq!(client.get_global_debt(), 850_000);
    }

    #[test]
    fn test_timelocked_governor_ceiling_update() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_sequence_number(100);

        let contract_id = env.register(DebtCeilingController, ());
        let client = DebtCeilingControllerClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let asset = Address::generate(&env);

        client.initialize(&admin, &5_000_000, &25);
        client.set_collateral_ceiling(&admin, &asset, &100_000);

        let effective = client.propose_ceiling_change(&admin, &asset, &250_000);
        assert_eq!(effective, 125);

        // Executing before ledger 125 fails
        env.ledger().set_sequence_number(124);
        assert_eq!(
            client.try_execute_ceiling_change(&asset),
            Err(Ok(DebtCeilingError::TimelockNotExpired))
        );

        // Executing at ledger 125 succeeds
        env.ledger().set_sequence_number(125);
        assert_eq!(client.execute_ceiling_change(&asset), 250_000);
        assert_eq!(client.get_collateral_profile(&asset).ceiling, 250_000);
    }
}

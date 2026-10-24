#![no_std]

mod ttl_extension;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env,
};

pub use ttl_extension::{
    BALANCE_TTL_EXTEND_TO, BALANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO,
    INSTANCE_TTL_THRESHOLD, LEDGERS_PER_DAY,
};
use ttl_extension::{extend_balance, extend_balance_if_present, extend_instance};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Asset,
    TotalDeposits,
    Balance(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RentManagerError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    InsufficientBalance = 4,
    ArithmeticOverflow = 5,
    NoRestorableBalance = 6,
}

#[contract]
pub struct RentManager;

#[contractimpl]
impl RentManager {
    /// Initialize the vault. The token address is the asset held for deposits.
    pub fn initialize(
        env: Env,
        admin: Address,
        asset: Address,
    ) -> Result<(), RentManagerError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(RentManagerError::AlreadyInitialized);
        }
        admin.require_auth();
        extend_instance(&env);
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::Asset, &asset);
        storage.set(&DataKey::TotalDeposits, &0i128);
        Ok(())
    }

    /// Deposit approved tokens into the vault and credit the caller's balance.
    pub fn deposit(
        env: Env,
        depositor: Address,
        amount: i128,
    ) -> Result<(), RentManagerError> {
        extend_instance(&env);
        depositor.require_auth();
        if amount <= 0 {
            return Err(RentManagerError::InvalidAmount);
        }

        let asset = Self::asset(&env)?;
        let balance = Self::read_balance(&env, &depositor);
        let next_balance = balance
            .checked_add(amount)
            .ok_or(RentManagerError::ArithmeticOverflow)?;
        let total = Self::total(&env)?;
        let next_total = total
            .checked_add(amount)
            .ok_or(RentManagerError::ArithmeticOverflow)?;

        token::Client::new(&env, &asset).transfer_from(
            &env.current_contract_address(),
            &depositor,
            &env.current_contract_address(),
            &amount,
        );
        Self::write_balance(&env, &depositor, next_balance);
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposits, &next_total);
        Ok(())
    }

    /// Withdraw deposited tokens. Only the balance entry for this user is
    /// extended; unrelated users' persistent entries are not touched.
    pub fn withdraw(
        env: Env,
        depositor: Address,
        amount: i128,
    ) -> Result<(), RentManagerError> {
        extend_instance(&env);
        depositor.require_auth();
        if amount <= 0 {
            return Err(RentManagerError::InvalidAmount);
        }

        let balance = Self::read_balance(&env, &depositor);
        if amount > balance {
            return Err(RentManagerError::InsufficientBalance);
        }
        let total = Self::total(&env)?;
        let next_total = total
            .checked_sub(amount)
            .ok_or(RentManagerError::ArithmeticOverflow)?;
        let asset = Self::asset(&env)?;

        Self::write_balance(&env, &depositor, balance - amount);
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposits, &next_total);
        token::Client::new(&env, &asset).transfer(
            &env.current_contract_address(),
            &depositor,
            &amount,
        );
        Ok(())
    }

    /// Read a user's balance, extending only that user's persistent entry.
    pub fn balance(env: Env, owner: Address) -> i128 {
        extend_instance(&env);
        Self::read_balance(&env, &owner)
    }

    /// Return aggregate deposits stored in the contract instance.
    pub fn total_deposits(env: Env) -> Result<i128, RentManagerError> {
        extend_instance(&env);
        Self::total(&env)
    }

    /// Refresh a user's balance TTL after the transaction has restored its
    /// archived ledger entry through Soroban's RestoreFootprint operation.
    ///
    /// This method cannot itself revive archived ledger entries. The restore
    /// operation, and its rent resource fee, are transaction-level operations.
    /// It is permissionless so keepers can pay the transaction fee for users.
    pub fn restore(env: Env, owner: Address) -> Result<i128, RentManagerError> {
        extend_instance(&env);
        if !extend_balance_if_present(&env, &owner) {
            return Err(RentManagerError::NoRestorableBalance);
        }
        env.storage()
            .persistent()
            .get(&DataKey::Balance(owner))
            .ok_or(RentManagerError::NoRestorableBalance)
    }
}

impl RentManager {
    fn asset(env: &Env) -> Result<Address, RentManagerError> {
        env.storage()
            .instance()
            .get(&DataKey::Asset)
            .ok_or(RentManagerError::NotInitialized)
    }

    fn total(env: &Env) -> Result<i128, RentManagerError> {
        env.storage()
            .instance()
            .get(&DataKey::TotalDeposits)
            .ok_or(RentManagerError::NotInitialized)
    }

    fn read_balance(env: &Env, owner: &Address) -> i128 {
        if !extend_balance_if_present(env, owner) {
            return 0;
        }
        env.storage()
            .persistent()
            .get(&DataKey::Balance(owner.clone()))
            .unwrap_or(0)
    }

    fn write_balance(env: &Env, owner: &Address, amount: i128) {
        let key = DataKey::Balance(owner.clone());
        env.storage().persistent().set(&key, &amount);
        extend_balance(env, owner);
    }
}

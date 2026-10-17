use soroban_sdk::{contracterror, contracttype, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DebtCeilingError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    CeilingExceeded = 5,
    DebtUnderflow = 6,
    TimelockNotExpired = 7,
    NoPendingProposal = 8,
    CollateralFrozen = 9,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralRiskProfile {
    pub ceiling: i128,
    pub current_debt: i128,
    pub frozen: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCeilingAdjustment {
    pub proposed_ceiling: i128,
    pub effective_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Admin,
    TimelockDelay,
    GlobalDebtCeiling,
    GlobalCurrentDebt,
    Collateral(Address),
    PendingAdjustment(Address),
}

pub fn read_profile(env: &Env, asset: &Address) -> CollateralRiskProfile {
    env.storage()
        .persistent()
        .get(&StorageKey::Collateral(asset.clone()))
        .unwrap_or(CollateralRiskProfile {
            ceiling: 0,
            current_debt: 0,
            frozen: false,
        })
}

pub fn write_profile(env: &Env, asset: &Address, profile: &CollateralRiskProfile) {
    env.storage()
        .persistent()
        .set(&StorageKey::Collateral(asset.clone()), profile);
}

pub fn validate_and_apply_mint(
    env: &Env,
    asset: &Address,
    amount: i128,
) -> Result<CollateralRiskProfile, DebtCeilingError> {
    if amount <= 0 {
        return Err(DebtCeilingError::InvalidAmount);
    }

    let mut profile = read_profile(env, asset);
    if profile.frozen {
        return Err(DebtCeilingError::CollateralFrozen);
    }

    let next_collateral_debt = profile
        .current_debt
        .checked_add(amount)
        .ok_or(DebtCeilingError::CeilingExceeded)?;
    if next_collateral_debt > profile.ceiling {
        return Err(DebtCeilingError::CeilingExceeded);
    }

    let global_ceiling: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::GlobalDebtCeiling)
        .unwrap_or(0);
    let global_debt: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::GlobalCurrentDebt)
        .unwrap_or(0);
    let next_global_debt = global_debt
        .checked_add(amount)
        .ok_or(DebtCeilingError::CeilingExceeded)?;
    if next_global_debt > global_ceiling {
        return Err(DebtCeilingError::CeilingExceeded);
    }

    profile.current_debt = next_collateral_debt;
    write_profile(env, asset, &profile);
    env.storage()
        .instance()
        .set(&StorageKey::GlobalCurrentDebt, &next_global_debt);

    Ok(profile)
}

pub fn validate_and_apply_repay(
    env: &Env,
    asset: &Address,
    amount: i128,
) -> Result<CollateralRiskProfile, DebtCeilingError> {
    if amount <= 0 {
        return Err(DebtCeilingError::InvalidAmount);
    }

    let mut profile = read_profile(env, asset);
    if profile.current_debt < amount {
        return Err(DebtCeilingError::DebtUnderflow);
    }

    let global_debt: i128 = env
        .storage()
        .instance()
        .get(&StorageKey::GlobalCurrentDebt)
        .unwrap_or(0);
    let next_global_debt = global_debt
        .checked_sub(amount)
        .ok_or(DebtCeilingError::DebtUnderflow)?;

    profile.current_debt -= amount;
    write_profile(env, asset, &profile);
    env.storage()
        .instance()
        .set(&StorageKey::GlobalCurrentDebt, &next_global_debt);

    Ok(profile)
}

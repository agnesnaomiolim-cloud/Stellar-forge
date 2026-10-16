//! Indexed checkpoints: same-ledger writes coalesce; historical reads use binary search.
use soroban_sdk::{contracttype, Address, Env};
use crate::{Error, Key};

/// Independent historical series.
#[contracttype]
#[derive(Clone)]
pub enum Series { Balance(Address), Votes(Address), Supply }

/// Value at the end of a ledger.
#[contracttype]
#[derive(Clone)]
pub struct Checkpoint { pub ledger: u32, pub value: i128 }

pub(crate) fn write(env: &Env, series: Series, value: i128) -> Result<(), Error> {
    let count_key = Key::Count(series.clone());
    let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
    let ledger = env.ledger().sequence();
    let index = if count > 0 {
        let last: Checkpoint = env.storage().persistent()
            .get(&Key::Point(series.clone(), count - 1)).ok_or(Error::MissingHistory)?;
        if last.ledger == ledger { count - 1 } else { count }
    } else { 0 };
    let point_key = Key::Point(series, index);
    env.storage().persistent().set(&point_key, &Checkpoint { ledger, value });
    crate::retain(env, &point_key);
    if index == count {
        env.storage().persistent().set(&count_key, &count.checked_add(1).ok_or(Error::Overflow)?);
    }
    crate::retain(env, &count_key);
    Ok(())
}

pub(crate) fn lookup(env: &Env, series: Series, ledger: u32) -> Result<i128, Error> {
    if ledger >= env.ledger().sequence() { return Err(Error::UnclosedLedger); }
    let mut low = 0_u32;
    let mut high: u32 = env.storage().persistent().get(&Key::Count(series.clone())).unwrap_or(0);
    while low < high {
        let mid = low + (high - low) / 2;
        let point: Checkpoint = env.storage().persistent()
            .get(&Key::Point(series.clone(), mid)).ok_or(Error::MissingHistory)?;
        if point.ledger <= ledger { low = mid + 1; } else { high = mid; }
    }
    if low == 0 { return Ok(0); }
    let point: Checkpoint = env.storage().persistent()
        .get(&Key::Point(series, low - 1)).ok_or(Error::MissingHistory)?;
    Ok(point.value)
}

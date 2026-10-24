use soroban_sdk::{Address, Env};

pub const LEDGERS_PER_DAY: u32 = 17_280;
pub const INSTANCE_TTL_THRESHOLD: u32 = 5 * LEDGERS_PER_DAY;
pub const INSTANCE_TTL_EXTEND_TO: u32 = 30 * LEDGERS_PER_DAY;
pub const BALANCE_TTL_THRESHOLD: u32 = 5 * LEDGERS_PER_DAY;
pub const BALANCE_TTL_EXTEND_TO: u32 = 30 * LEDGERS_PER_DAY;

pub(crate) fn extend_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

pub(crate) fn extend_balance_if_present(env: &Env, owner: &Address) -> bool {
    let key = crate::DataKey::Balance(owner.clone());
    let storage = env.storage().persistent();
    if !storage.has(&key) {
        return false;
    }
    storage.extend_ttl(
        &key,
        BALANCE_TTL_THRESHOLD,
        BALANCE_TTL_EXTEND_TO,
    );
    true
}

pub(crate) fn extend_balance(env: &Env, owner: &Address) {
    let key = crate::DataKey::Balance(owner.clone());
    env.storage().persistent().extend_ttl(
        &key,
        BALANCE_TTL_THRESHOLD,
        BALANCE_TTL_EXTEND_TO,
    );
}

use soroban_sdk::{contracterror, contracttype, BytesN, Env};

pub const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
pub const INSTANCE_BUMP_AMOUNT: u32 = 518_400;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RelayerError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    UnauthorizedRelayer = 3,
    DuplicateRelayerVote = 4,
    InsufficientSignatures = 5,
    InvalidDomainSeparator = 6,
    InvalidSequenceNonce = 7,
    MessageAlreadyExecuted = 8,
}

#[contracttype]
#[derive(Clone)]
pub enum NonceKey {
    NextExpectedNonce(u32),
    ExecutedDigest(BytesN<32>),
}

pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

pub fn verify_and_advance_nonce(
    env: &Env,
    source_chain_id: u32,
    nonce: u64,
    payload_digest: &BytesN<32>,
) -> Result<(), RelayerError> {
    if env
        .storage()
        .instance()
        .has(&NonceKey::ExecutedDigest(payload_digest.clone()))
    {
        return Err(RelayerError::MessageAlreadyExecuted);
    }

    let expected: u64 = env
        .storage()
        .instance()
        .get(&NonceKey::NextExpectedNonce(source_chain_id))
        .unwrap_or(1);

    if nonce != expected {
        return Err(RelayerError::InvalidSequenceNonce);
    }

    env.storage().instance().set(
        &NonceKey::NextExpectedNonce(source_chain_id),
        &(expected + 1),
    );
    env.storage()
        .instance()
        .set(&NonceKey::ExecutedDigest(payload_digest.clone()), &true);
    extend_instance_ttl(env);

    Ok(())
}

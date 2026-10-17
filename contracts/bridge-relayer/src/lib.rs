#![no_std]

pub mod nonce;

use nonce::{extend_instance_ttl, verify_and_advance_nonce, RelayerError};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Bytes, BytesN, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossChainEnvelope {
    pub domain_separator: BytesN<32>,
    pub source_chain_id: u32,
    pub nonce: u64,
    pub target_contract: Address,
    pub target_fn: Symbol,
    pub payload: Bytes,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayerConfig {
    pub admin: Address,
    pub domain_separator: BytesN<32>,
    pub relayers: Vec<Address>,
    pub threshold: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Config,
    DispatchedCount,
}

#[contract]
pub struct BridgeRelayerContract;

#[contractimpl]
impl BridgeRelayerContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        domain_separator: BytesN<32>,
        relayers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), RelayerError> {
        if env.storage().instance().has(&StorageKey::Config) {
            return Err(RelayerError::AlreadyInitialized);
        }
        if threshold == 0 || relayers.len() < threshold {
            return Err(RelayerError::InsufficientSignatures);
        }
        admin.require_auth();
        env.storage().instance().set(
            &StorageKey::Config,
            &RelayerConfig {
                admin,
                domain_separator,
                relayers,
                threshold,
            },
        );
        env.storage()
            .instance()
            .set(&StorageKey::DispatchedCount, &0u64);
        extend_instance_ttl(&env);
        Ok(())
    }

    pub fn verify_and_dispatch(
        env: Env,
        signers: Vec<Address>,
        envelope: CrossChainEnvelope,
    ) -> Result<BytesN<32>, RelayerError> {
        let config: RelayerConfig = env
            .storage()
            .instance()
            .get(&StorageKey::Config)
            .ok_or(RelayerError::NotInitialized)?;

        if envelope.domain_separator != config.domain_separator {
            return Err(RelayerError::InvalidDomainSeparator);
        }

        if signers.len() < config.threshold {
            return Err(RelayerError::InsufficientSignatures);
        }

        let mut seen = Vec::new(&env);
        for signer in signers.iter() {
            signer.require_auth();
            if !config.relayers.contains(&signer) {
                return Err(RelayerError::UnauthorizedRelayer);
            }
            if seen.contains(&signer) {
                return Err(RelayerError::DuplicateRelayerVote);
            }
            seen.push_back(signer);
        }

        let digest: BytesN<32> = env.crypto().sha256(&envelope.payload).into();
        verify_and_advance_nonce(&env, envelope.source_chain_id, envelope.nonce, &digest)?;

        let count: u64 = env
            .storage()
            .instance()
            .get(&StorageKey::DispatchedCount)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&StorageKey::DispatchedCount, &(count + 1));

        Ok(digest)
    }

    pub fn get_dispatched_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&StorageKey::DispatchedCount)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_multisig_threshold_replay_protection_and_sequential_nonces() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(BridgeRelayerContract, ());
        let client = BridgeRelayerContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let r1 = Address::generate(&env);
        let r2 = Address::generate(&env);
        let r3 = Address::generate(&env);
        let target = Address::generate(&env);
        let domain = BytesN::from_array(&env, &[7u8; 32]);

        let mut relayers = Vec::new(&env);
        relayers.push_back(r1.clone());
        relayers.push_back(r2.clone());
        relayers.push_back(r3.clone());

        client.initialize(&admin, &domain, &relayers, &2);

        let env_msg1 = CrossChainEnvelope {
            domain_separator: domain.clone(),
            source_chain_id: 1,
            nonce: 1,
            target_contract: target.clone(),
            target_fn: Symbol::new(&env, "mint"),
            payload: Bytes::from_slice(&env, b"transfer-100"),
        };

        // 1 signer (< threshold 2) fails
        let mut one_signer = Vec::new(&env);
        one_signer.push_back(r1.clone());
        assert_eq!(
            client.try_verify_and_dispatch(&one_signer, &env_msg1),
            Err(Ok(RelayerError::InsufficientSignatures))
        );

        // 2 valid relayers succeed
        let mut two_signers = Vec::new(&env);
        two_signers.push_back(r1.clone());
        two_signers.push_back(r2.clone());
        let digest = client.verify_and_dispatch(&two_signers, &env_msg1);
        assert_eq!(digest.len(), 32);
        assert_eq!(client.get_dispatched_count(), 1);

        // Replay attack with same digest or nonce fails immediately
        assert_eq!(
            client.try_verify_and_dispatch(&two_signers, &env_msg1),
            Err(Ok(RelayerError::MessageAlreadyExecuted))
        );

        // Out-of-order nonce (nonce=3 instead of 2) fails
        let env_msg_bad_nonce = CrossChainEnvelope {
            domain_separator: domain,
            source_chain_id: 1,
            nonce: 3,
            target_contract: target,
            target_fn: Symbol::new(&env, "mint"),
            payload: Bytes::from_slice(&env, b"transfer-200"),
        };
        assert_eq!(
            client.try_verify_and_dispatch(&two_signers, &env_msg_bad_nonce),
            Err(Ok(RelayerError::InvalidSequenceNonce))
        );
    }
}

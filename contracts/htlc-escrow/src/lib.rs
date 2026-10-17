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

#![no_std]

pub mod crypto;

use crypto::HashAlgorithm;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Bytes, BytesN, Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum HtlcError {
    EscrowAlreadyExists = 1,
    EscrowNotFound = 2,
    AlreadySettled = 3,
    Unauthorized = 4,
    TimelockExpired = 5,
    TimelockNotExpired = 6,
    InvalidPreimage = 7,
    EmptyAssetBundle = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Lock(BytesN<32>),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetLeg {
    pub token: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Escrow {
    pub sender: Address,
    pub receiver: Address,
    pub assets: Vec<AssetLeg>,
    pub timelock: u64,
    pub hashlock: BytesN<32>,
    pub algorithm: HashAlgorithm,
    pub settled: bool,
}

#[contract]
pub struct HtlcContract;

#[contractimpl]
impl HtlcContract {
    pub fn deposit_bundle(
        env: Env,
        sender: Address,
        receiver: Address,
        assets: Vec<AssetLeg>,
        hashlock: BytesN<32>,
        algorithm: HashAlgorithm,
        timelock: u64,
    ) -> Result<(), HtlcError> {
        sender.require_auth();
        if assets.is_empty() {
            return Err(HtlcError::EmptyAssetBundle);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Lock(hashlock.clone()))
        {
            return Err(HtlcError::EscrowAlreadyExists);
        }

        let contract_addr = env.current_contract_address();
        for leg in assets.iter() {
            let token_client = token::Client::new(&env, &leg.token);
            token_client.transfer(&sender, &contract_addr, &leg.amount);
        }

        let escrow = Escrow {
            sender,
            receiver,
            assets,
            timelock,
            hashlock: hashlock.clone(),
            algorithm,
            settled: false,
        };
        env.storage().persistent().set(&DataKey::Lock(hashlock), &escrow);
        Ok(())
    }

    pub fn claim(
        env: Env,
        receiver: Address,
        hashlock: BytesN<32>,
        preimage: Bytes,
    ) -> Result<(), HtlcError> {
        receiver.require_auth();

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(hashlock.clone()))
            .ok_or(HtlcError::EscrowNotFound)?;

        if escrow.settled {
            return Err(HtlcError::AlreadySettled);
        }
        if escrow.receiver != receiver {
            return Err(HtlcError::Unauthorized);
        }
        if env.ledger().timestamp() >= escrow.timelock {
            return Err(HtlcError::TimelockExpired);
        }
        if !crypto::verify_hash_with_algo(&env, escrow.algorithm, &preimage, &escrow.hashlock) {
            return Err(HtlcError::InvalidPreimage);
        }

        escrow.settled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Lock(hashlock), &escrow);

        let contract_addr = env.current_contract_address();
        for leg in escrow.assets.iter() {
            let token_client = token::Client::new(&env, &leg.token);
            token_client.transfer(&contract_addr, &receiver, &leg.amount);
        }
        Ok(())
    }

    pub fn refund(env: Env, sender: Address, hashlock: BytesN<32>) -> Result<(), HtlcError> {
        sender.require_auth();

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&DataKey::Lock(hashlock.clone()))
            .ok_or(HtlcError::EscrowNotFound)?;

        if escrow.settled {
            return Err(HtlcError::AlreadySettled);
        }
        if escrow.sender != sender {
            return Err(HtlcError::Unauthorized);
        }
        if env.ledger().timestamp() < escrow.timelock {
            return Err(HtlcError::TimelockNotExpired);
        }

        escrow.settled = true;
        env.storage()
            .persistent()
            .set(&DataKey::Lock(hashlock), &escrow);

        let contract_addr = env.current_contract_address();
        for leg in escrow.assets.iter() {
            let token_client = token::Client::new(&env, &leg.token);
            token_client.transfer(&contract_addr, &sender, &leg.amount);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::vec;

    #[test]
    fn test_multi_asset_htlc_claim_and_timeout_refund() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.timestamp = 1_000;
        });

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        let token1_addr = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let token2_addr = env.register_stellar_asset_contract_v2(admin).address();

        token::StellarAssetClient::new(&env, &token1_addr).mint(&sender, &1_000);
        token::StellarAssetClient::new(&env, &token2_addr).mint(&sender, &2_000);

        let contract_id = env.register(HtlcContract, ());
        let client = HtlcContractClient::new(&env, &contract_id);

        // 1. Happy-path multi-asset claim with SHA-256 preimage before timelock
        let secret1 = Bytes::from_slice(&env, b"atomic-cross-chain-secret-1");
        let hashlock1: BytesN<32> = env.crypto().sha256(&secret1).into();
        let bundle1 = vec![
            &env,
            AssetLeg {
                token: token1_addr.clone(),
                amount: 400,
            },
            AssetLeg {
                token: token2_addr.clone(),
                amount: 800,
            },
        ];
        client.deposit_bundle(
            &sender,
            &receiver,
            &bundle1,
            &hashlock1,
            &HashAlgorithm::Sha256,
            &2_000,
        );
        client.claim(&receiver, &hashlock1, &secret1);
        assert_eq!(token::Client::new(&env, &token1_addr).balance(&receiver), 400);
        assert_eq!(token::Client::new(&env, &token2_addr).balance(&receiver), 800);

        // 2. Timeout refund path with Keccak-256 hashlock after timelock expiry
        let secret2 = Bytes::from_slice(&env, b"atomic-cross-chain-secret-2");
        let hashlock2: BytesN<32> = env.crypto().keccak256(&secret2).into();
        let bundle2 = vec![
            &env,
            AssetLeg {
                token: token1_addr.clone(),
                amount: 600,
            },
        ];
        client.deposit_bundle(
            &sender,
            &receiver,
            &bundle2,
            &hashlock2,
            &HashAlgorithm::Keccak256,
            &2_000,
        );

        // Early refund before timestamp 2000 fails
        assert_eq!(
            client.try_refund(&sender, &hashlock2),
            Err(Ok(HtlcError::TimelockNotExpired))
        );

        // Advance ledger past timelock -> refund succeeds
        env.ledger().with_mut(|li| {
            li.timestamp = 2_500;
        });
        client.refund(&sender, &hashlock2);
        assert_eq!(token::Client::new(&env, &token1_addr).balance(&sender), 600);
    }
}

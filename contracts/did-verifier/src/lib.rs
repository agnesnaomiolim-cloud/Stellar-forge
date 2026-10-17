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
pub mod revocation;
pub mod schema;

use revocation::{MerkleProof, RevocationRegistry};
use schema::{CredentialClaim, Schema};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Bytes, BytesN, Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Error {
    ParseError = 1,
    InvalidSignature = 2,
    Expired = 3,
    Revoked = 4,
    SchemaViolation = 5,
    NotYetValid = 6,
}

#[contracttype]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyType {
    Ed25519,
    Secp256k1(u32),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiableCredentialPayload {
    pub credential_id: Bytes,
    pub issuer_did: Bytes,
    pub issuance_timestamp: u64,
    pub expiration_timestamp: Option<u64>,
    pub claims: Vec<CredentialClaim>,
    pub canonical_message: Bytes,
    pub signature: BytesN<64>,
}

#[contract]
pub struct DidVerifier;

#[contractimpl]
impl DidVerifier {
    pub fn verify_credential(
        env: Env,
        credential: VerifiableCredentialPayload,
        issuer_public_key: Bytes,
        key_type: KeyType,
        revocation_registry: Option<RevocationRegistry>,
        revocation_proof: Option<MerkleProof>,
        schema: Option<Schema>,
    ) -> Result<bool, Error> {
        let now = env.ledger().timestamp();
        if credential.issuance_timestamp > now {
            return Err(Error::NotYetValid);
        }
        if let Some(exp) = credential.expiration_timestamp {
            if now >= exp {
                return Err(Error::Expired);
            }
        }

        if let Some(reg) = revocation_registry {
            if revocation::is_revoked(
                &env,
                &reg,
                &credential.credential_id,
                revocation_proof.as_ref(),
            ) {
                return Err(Error::Revoked);
            }
        }

        if let Some(s) = schema {
            if !schema::validate_schema(&credential.claims, &s) {
                return Err(Error::SchemaViolation);
            }
        }

        let valid_sig = match key_type {
            KeyType::Ed25519 => {
                if issuer_public_key.len() != 32 {
                    return Err(Error::InvalidSignature);
                }
                let mut pk = [0u8; 32];
                issuer_public_key.copy_into_slice(&mut pk);
                let pk_bytes = BytesN::from_array(&env, &pk);
                crypto::verify_ed25519(
                    &env,
                    &pk_bytes,
                    &credential.canonical_message,
                    &credential.signature,
                )
            }
            KeyType::Secp256k1(recovery_id) => {
                if issuer_public_key.len() != 65 {
                    return Err(Error::InvalidSignature);
                }
                let mut pk = [0u8; 65];
                issuer_public_key.copy_into_slice(&mut pk);
                let pk_bytes = BytesN::from_array(&env, &pk);
                crypto::verify_secp256k1(
                    &env,
                    &pk_bytes,
                    &credential.canonical_message,
                    &credential.signature,
                    recovery_id,
                )
            }
        };

        if !valid_sig {
            return Err(Error::InvalidSignature);
        }

        Ok(true)
    }
}

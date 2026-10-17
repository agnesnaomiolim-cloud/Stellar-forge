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

use did_verifier::revocation::{MerkleProof, RevocationRegistry};
use did_verifier::schema::{CredentialClaim, RequiredClaim, Schema};
use did_verifier::{DidVerifier, DidVerifierClient, Error, KeyType, VerifiableCredentialPayload};
use soroban_sdk::testutils::Ledger;
use soroban_sdk::{symbol_short, vec, Bytes, BytesN, Env};

#[test]
fn test_w3c_verifiable_credential_expiry_revocation_and_schema_enforcement() {
    let env = Env::default();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });

    let contract_id = env.register(DidVerifier, ());
    let client = DidVerifierClient::new(&env, &contract_id);

    let cred_id = Bytes::from_slice(&env, b"urn:uuid:w3c-vc-001");
    let leaf_hash: BytesN<32> = env.crypto().sha256(&cred_id).into();
    let sibling = BytesN::from_array(&env, &[0x11u8; 32]);
    let mut combined = Bytes::new(&env);
    combined.append(&Bytes::from_array(&env, &leaf_hash.to_array()));
    combined.append(&Bytes::from_array(&env, &sibling.to_array()));
    let merkle_root: BytesN<32> = env.crypto().sha256(&combined).into();

    let payload = VerifiableCredentialPayload {
        credential_id: cred_id,
        issuer_did: Bytes::from_slice(&env, b"did:stellar:GISSUER"),
        issuance_timestamp: 1_690_000_000,
        expiration_timestamp: Some(1_800_000_000),
        claims: vec![
            &env,
            CredentialClaim {
                key: symbol_short!("kyc_lvl"),
                value_u64: 3,
            },
        ],
        canonical_message: Bytes::from_slice(&env, b"w3c-vc-payload"),
        signature: BytesN::from_array(&env, &[0u8; 64]),
    };

    let pubkey = Bytes::from_slice(&env, &[0u8; 32]);

    // 1. Revoked credential via Merkle inclusion proof is rejected with Error::Revoked
    let proof = MerkleProof {
        siblings: vec![&env, sibling],
        is_left: vec![&env, false],
    };
    let revoked_res = client.try_verify_credential(
        &payload,
        &pubkey,
        &KeyType::Ed25519,
        &Some(RevocationRegistry::MerkleRoot(merkle_root)),
        &Some(proof),
        &None,
    );
    assert_eq!(revoked_res, Err(Ok(Error::Revoked)));

    // 2. Schema violation (requires kyc_lvl >= 5, credential has 3) -> Error::SchemaViolation
    let strict_schema = Schema {
        required_claims: vec![
            &env,
            RequiredClaim {
                key: symbol_short!("kyc_lvl"),
                min_value_u64: 5,
            },
        ],
    };
    let schema_res = client.try_verify_credential(
        &payload,
        &pubkey,
        &KeyType::Ed25519,
        &None,
        &None,
        &Some(strict_schema),
    );
    assert_eq!(schema_res, Err(Ok(Error::SchemaViolation)));

    // 3. Expired credential -> Error::Expired
    env.ledger().with_mut(|li| {
        li.timestamp = 1_900_000_000;
    });
    let exp_res =
        client.try_verify_credential(&payload, &pubkey, &KeyType::Ed25519, &None, &None, &None);
    assert_eq!(exp_res, Err(Ok(Error::Expired)));
}

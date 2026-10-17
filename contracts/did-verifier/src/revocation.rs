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

use soroban_sdk::{contracttype, Bytes, BytesN, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevocationRegistry {
    Bitmap(Bytes),
    MerkleRoot(BytesN<32>),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MerkleProof {
    pub siblings: Vec<BytesN<32>>,
    pub is_left: Vec<bool>,
}

pub fn is_revoked(
    env: &Env,
    registry: &RevocationRegistry,
    credential_id: &Bytes,
    proof: Option<&MerkleProof>,
) -> bool {
    match registry {
        RevocationRegistry::Bitmap(bitmap) => {
            if bitmap.is_empty() {
                return false;
            }
            let hash: BytesN<32> = env.crypto().sha256(credential_id).into();
            let arr = hash.to_array();
            let index = u32::from_be_bytes([arr[0], arr[1], arr[2], arr[3]]);
            let total_bits = bitmap.len() * 8;
            let bit_pos = index % total_bits;
            let byte_pos = bit_pos / 8;
            let bit_in_byte = bit_pos % 8;
            let byte = bitmap.get(byte_pos).unwrap_or(0);
            ((byte >> (7 - bit_in_byte)) & 1) == 1
        }
        RevocationRegistry::MerkleRoot(root) => {
            if let Some(p) = proof {
                verify_merkle_inclusion(env, credential_id, root, p)
            } else {
                false
            }
        }
    }
}

fn verify_merkle_inclusion(
    env: &Env,
    credential_id: &Bytes,
    root: &BytesN<32>,
    proof: &MerkleProof,
) -> bool {
    if proof.siblings.len() != proof.is_left.len() {
        return false;
    }
    let mut current: BytesN<32> = env.crypto().sha256(credential_id).into();
    for i in 0..proof.siblings.len() {
        let sibling = proof.siblings.get(i).unwrap();
        let sibling_is_left = proof.is_left.get(i).unwrap();
        let mut combined = Bytes::new(env);
        if sibling_is_left {
            combined.append(&Bytes::from_array(env, &sibling.to_array()));
            combined.append(&Bytes::from_array(env, &current.to_array()));
        } else {
            combined.append(&Bytes::from_array(env, &current.to_array()));
            combined.append(&Bytes::from_array(env, &sibling.to_array()));
        }
        current = env.crypto().sha256(&combined).into();
    }
    &current == root
}
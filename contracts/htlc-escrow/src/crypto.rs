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

use soroban_sdk::{contracttype, Bytes, BytesN, Env};

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HashAlgorithm {
    Sha256,
    Keccak256,
}

pub fn verify_hash_with_algo(
    env: &Env,
    algo: HashAlgorithm,
    preimage: &Bytes,
    expected_hash: &BytesN<32>,
) -> bool {
    let computed: BytesN<32> = match algo {
        HashAlgorithm::Sha256 => env.crypto().sha256(preimage).into(),
        HashAlgorithm::Keccak256 => env.crypto().keccak256(preimage).into(),
    };
    &computed == expected_hash
}

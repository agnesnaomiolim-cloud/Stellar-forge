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

use soroban_sdk::{Bytes, BytesN, Env};

pub fn verify_ed25519(
    env: &Env,
    public_key: &BytesN<32>,
    message: &Bytes,
    signature: &BytesN<64>,
) -> bool {
    env.crypto().ed25519_verify(public_key, message, signature);
    true
}

pub fn verify_secp256k1(
    env: &Env,
    expected_uncompressed_pubkey: &BytesN<65>,
    message: &Bytes,
    signature: &BytesN<64>,
    recovery_id: u32,
) -> bool {
    if recovery_id > 3 {
        return false;
    }
    let digest = env.crypto().keccak256(message);
    let recovered = env
        .crypto()
        .secp256k1_recover(&digest, signature, recovery_id);
    &recovered == expected_uncompressed_pubkey
}

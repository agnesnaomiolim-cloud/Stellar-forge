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
use soroban_sdk::{Env, Bytes, BytesN};

pub fn verify_ed25519(env: &Env, public_key: &BytesN
<32>, message: &Bytes, signature: &BytesN
<64>) -> bool {
    env.verify_ed25519_sig(public_key.clone(), message.clone(), signature.clone())
}

pub fn verify_secp256k1(env: &Env, public_key: &Bytes, message: &Bytes, signature: &BytesN:<64>) -> bool {
    // Try both recovery ids 0 and 1.
    for recovery_id in 0..2 {
        let recovered = env.recover_ecdsa_secp256k1(message.clone(), signature.clone(), recovery_id);
        if recovered.len() == 65 && public_key.len() == 65 && recovered.to_vec() == public_key.to_vec() {
            return true;
        }
    }
    false
}

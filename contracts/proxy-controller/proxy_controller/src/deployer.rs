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

/// Uploads WASM bytecode to the ledger and returns its content hash.
///
/// The hash returned here is what gets embedded in an [`crate::PendingUpgrade`];
/// the bytecode itself does not become live until [`apply`] is invoked after
/// the timelock has elapsed.
pub fn upload(env: &Env, wasm: Bytes) -> BytesN<32> {
    env.deployer().upload_contract_wasm(wasm)
}

/// Swaps the executable of the currently executing contract to `wasm_hash`.
///
/// Per Soroban semantics, a contract can only ever replace its own code, so
/// this must be called from within the contract being upgraded (never on
/// behalf of another contract address). The swap does not take effect until
/// the current top-level invocation finishes executing, so code already
/// running under the old Wasm keeps running under the old Wasm.
///
/// Existing contract storage (instance, persistent and temporary entries) is
/// left completely untouched by this call: Soroban keys storage by contract
/// address, not by the code currently installed at that address, so state
/// automatically survives the swap as long as the new Wasm uses storage keys
/// that are compatible with what is already there. See the storage-layout
/// guidance in `contracts/proxy-controller/README.md`.
pub fn apply(env: &Env, wasm_hash: BytesN<32>) {
    env.deployer().update_current_contract_wasm(wasm_hash);
}
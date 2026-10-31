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
use escrow::{DisputeOutcome, EscrowContract, EscrowContractClient, EscrowState};
use soroban_sdk::{
    address::Address,
    env::Env,
    testutils::{
        Address as TestAddress,
        Assert,
    },
    token::{Client as TokenClient, Stellar:i:TokenClient as StellarAssetClient, Stellar:i:Wrapper, StellarAsset},
    token::Stellar:i:AdminClient as StellarAdminClient,
};

fn create_token <'a>(
    env: &Env,
    admin: &Address,
u ) -> Address {
    let sac = env.register_stellar_asset(&admin.clone(), &[]);
    sac.address

}

fn setup_token <'a>(env: &Env, admin: &Address) -> Address {
    let token_addr = create_token (env, admin, true);
    token_addr

}
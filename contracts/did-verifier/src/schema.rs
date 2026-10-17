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

use soroban_sdk::{contracttype, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialClaim {
    pub key: Symbol,
    pub value_u64: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredClaim {
    pub key: Symbol,
    pub min_value_u64: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schema {
    pub required_claims: Vec<RequiredClaim>,
}

pub fn validate_schema(claims: &Vec<CredentialClaim>, schema: &Schema) -> bool {
    for req in schema.required_claims.iter() {
        let mut matched = false;
        for c in claims.iter() {
            if c.key == req.key && c.value_u64 >= req.min_value_u64 {
                matched = true;
                break;
            }
        }
        if !matched {
            return false;
        }
    }
    true
}

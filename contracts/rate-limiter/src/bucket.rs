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

use soroban_sdk::contracttype;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenBucket {
    pub tokens: u32,
    pub last_refill: u32,
}

impl TokenBucket {
    pub fn new(capacity: u32, current_ledger: u32) -> Self {
        Self {
            tokens: capacity,
            last_refill: current_ledger,
        }
    }

    pub fn consume(
        &mut self,
        amount: u32,
        capacity: u32,
        refill_rate: u32,
        current_ledger: u32,
    ) -> bool {
        let ledgers_elapsed = current_ledger.saturating_sub(self.last_refill);
        let added_tokens = ledgers_elapsed.saturating_mul(refill_rate);

        self.tokens = core::cmp::min(capacity, self.tokens.saturating_add(added_tokens));
        self.last_refill = current_ledger;

        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}

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
use soroban_sdk::Address;

#[derive(Clone)]
pub struct Allowance {
    pub operator: Address,
    pub limit: i128,
    pub spent: i128,
    pub window_start: u64,
    pub window_seconds: u64,
}

impl Allowance {
    pub fn new(
        operator: Address,
        limit: i128,
        window_start: u64,
        window_seconds: u64,
    ) -> Self {
        Self {
            operator,
            limit,
            spent: 0,
            window_start,
            window_seconds,
        }
    }

    pub fn available(&self, now: u64) -> i128 {
        if now >= self.window_start + self.window_seconds {
            self.limit
        } else {
            self.limit.saturating_sub(self.spent)
        }
    }

    pub fn consume(&mut self, amount: i128, now: u64) -> bool {
        if amount < 0 {
            return false;
        }

        if now >= self.window_start + self.window_seconds {
            self.window_start = now;
            self.spent = 0;
        }

        if self.spent.saturating_add(amount) > self.limit {
            return false;
        }

        self.spent += amount;
        true
    }
}
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurveShape {
    Linear,
    Quadratic,
    Sigmoid,
}

/// Calculates the exact reserve token cost to mint `amount` continuous tokens
/// starting from `total_supply` using 128-bit integer integral calculus:
/// - Linear: `P(s) = base_price + slope * s`, integral = `base_price * ds + slope * ((s1^2 - s0^2) / 2)`
/// - Quadratic: `P(s) = base_price + slope * s^2`, integral = `base_price * ds + slope * ((s1^3 - s0^3) / 3)`
/// - Sigmoid: S-curve inflection around `midpoint` with bounded asymptote.
pub fn calculate_purchase_cost(
    shape: CurveShape,
    total_supply: i128,
    amount: i128,
    base_price: i128,
    slope: i128,
) -> i128 {
    if amount <= 0 {
        return 0;
    }
    let s0 = total_supply.max(0);
    let s1 = s0.saturating_add(amount);
    match shape {
        CurveShape::Linear => {
            let base_term = base_price.saturating_mul(amount);
            let quad_diff = s1
                .saturating_mul(s1)
                .saturating_sub(s0.saturating_mul(s0));
            base_term.saturating_add(slope.saturating_mul(quad_diff) / 2)
        }
        CurveShape::Quadratic => {
            let base_term = base_price.saturating_mul(amount);
            let cube_diff = s1
                .saturating_mul(s1)
                .saturating_mul(s1)
                .saturating_sub(s0.saturating_mul(s0).saturating_mul(s0));
            base_term.saturating_add(slope.saturating_mul(cube_diff) / 3)
        }
        CurveShape::Sigmoid => {
            let mid = (s0 + s1) / 2;
            let num = slope.saturating_mul(mid);
            let denom = 1_000i128.saturating_add(mid);
            let unit_price = base_price.saturating_add(num / denom.max(1));
            unit_price.saturating_mul(amount)
        }
    }
}

/// Calculates the exact reserve token return when burning `amount` continuous tokens
/// from `total_supply`, preserving path-independence (`buy(X)` then `sell(X)` returns exact net reserve).
pub fn calculate_sale_return(
    shape: CurveShape,
    total_supply: i128,
    amount: i128,
    base_price: i128,
    slope: i128,
) -> i128 {
    if amount <= 0 || total_supply <= 0 || amount > total_supply {
        return 0;
    }
    let new_supply = total_supply - amount;
    calculate_purchase_cost(shape, new_supply, amount, base_price, slope)
}

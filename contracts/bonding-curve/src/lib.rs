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

#![no_std]

pub mod curve;

use curve::CurveShape;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

#[contract]
pub struct BondingCurveContract;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    CurveShape,
    BasePrice,
    Slope,
    FeeBps,
    ReserveBalance,
    TreasuryBalance,
    TotalSupply,
    Balance(Address),
}

#[contractimpl]
impl BondingCurveContract {
    pub fn initialize(
        env: Env,
        shape: CurveShape,
        base_price: i128,
        slope: i128,
        fee_bps: u32,
    ) {
        assert!(
            !env.storage().instance().has(&DataKey::CurveShape),
            "already initialized"
        );
        env.storage().instance().set(&DataKey::CurveShape, &shape);
        env.storage()
            .instance()
            .set(&DataKey::BasePrice, &base_price);
        env.storage().instance().set(&DataKey::Slope, &slope);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage()
            .instance()
            .set(&DataKey::ReserveBalance, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &0i128);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
    }

    pub fn buy(env: Env, buyer: Address, amount: i128, max_cost: i128) -> i128 {
        buyer.require_auth();
        assert!(amount > 0, "amount must be positive");

        let shape: CurveShape = env.storage().instance().get(&DataKey::CurveShape).unwrap();
        let base_price: i128 = env.storage().instance().get(&DataKey::BasePrice).unwrap();
        let slope: i128 = env.storage().instance().get(&DataKey::Slope).unwrap();
        let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap();
        let reserve_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap();
        let treasury_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap();
        let total_supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap();

        let reserve_cost =
            curve::calculate_purchase_cost(shape, total_supply, amount, base_price, slope);
        let fee = reserve_cost.saturating_mul(fee_bps as i128) / 10_000;
        let total_cost = reserve_cost.saturating_add(fee);
        assert!(total_cost <= max_cost, "slippage limit exceeded");

        let prev_bal: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(buyer.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::Balance(buyer), &(prev_bal + amount));
        env.storage().instance().set(
            &DataKey::ReserveBalance,
            &(reserve_balance + reserve_cost),
        );
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &(treasury_balance + fee));
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total_supply + amount));

        total_cost
    }

    pub fn sell(env: Env, seller: Address, amount: i128, min_return: i128) -> i128 {
        seller.require_auth();
        assert!(amount > 0, "amount must be positive");

        let prev_bal: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(seller.clone()))
            .unwrap_or(0);
        assert!(prev_bal >= amount, "insufficient token balance");

        let shape: CurveShape = env.storage().instance().get(&DataKey::CurveShape).unwrap();
        let base_price: i128 = env.storage().instance().get(&DataKey::BasePrice).unwrap();
        let slope: i128 = env.storage().instance().get(&DataKey::Slope).unwrap();
        let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap();
        let reserve_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap();
        let treasury_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap();
        let total_supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap();

        let gross_return =
            curve::calculate_sale_return(shape, total_supply, amount, base_price, slope);
        let fee = gross_return.saturating_mul(fee_bps as i128) / 10_000;
        let net_return = gross_return.saturating_sub(fee);
        assert!(net_return >= min_return, "slippage limit exceeded");

        env.storage()
            .persistent()
            .set(&DataKey::Balance(seller), &(prev_bal - amount));
        env.storage().instance().set(
            &DataKey::ReserveBalance,
            &(reserve_balance - gross_return),
        );
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &(treasury_balance + fee));
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total_supply - amount));

        net_return
    }

    pub fn get_reserve_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::ReserveBalance)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_bonding_curve_buy_and_sell_roundtrip_conservation() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(BondingCurveContract, ());
        let client = BondingCurveContractClient::new(&env, &contract_id);
        let trader = Address::generate(&env);

        // Initialize Linear curve: base_price = 100, slope = 2, fee_bps = 100 (1%)
        client.initialize(&CurveShape::Linear, &100, &2, &100);

        // Buy 100 tokens: integral = 100*100 + 2*(100^2)/2 = 10,000 + 10,000 = 20,000 (+200 fee = 20,200)
        let paid = client.buy(&trader, &100, &25_000);
        assert_eq!(paid, 20_200);
        assert_eq!(client.get_reserve_balance(), 20_000);

        // Sell all 100 tokens back: gross = 20,000, fee = 200, net_return = 19,800, reserve returns to 0
        let refunded = client.sell(&trader, &100, &19_500);
        assert_eq!(refunded, 19_800);
        assert_eq!(client.get_reserve_balance(), 0);
    }
}

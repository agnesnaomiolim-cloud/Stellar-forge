#![no_std]

pub mod engine;
pub mod queue;

use engine::{submit_order, MatchExecutionSummary};
use queue::{load_asks, load_bids, LimitOrder, OrderbookError, Side};
use soroban_sdk::{contract, contractimpl, Address, Env, Vec};

#[contract]
pub struct OrderbookContract;

#[contractimpl]
impl OrderbookContract {
    pub fn place_order(
        env: Env,
        maker: Address,
        side: Side,
        price: i128,
        qty: i128,
        max_matches: u32,
    ) -> Result<MatchExecutionSummary, OrderbookError> {
        maker.require_auth();
        submit_order(&env, maker, side, price, qty, max_matches)
    }

    pub fn get_asks(env: Env) -> Vec<LimitOrder> {
        load_asks(&env)
    }

    pub fn get_bids(env: Env) -> Vec<LimitOrder> {
        load_bids(&env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_50_crossed_limit_orders_and_partial_fills() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(OrderbookContract, ());
        let client = OrderbookContractClient::new(&env, &contract_id);

        let seller = Address::generate(&env);
        let buyer = Address::generate(&env);

        // Place 25 resting asks of 10 units each at price 100 (total 250 units)
        for _ in 0..25 {
            let res = client.place_order(&seller, &Side::Ask, &100, &10, &50);
            assert_eq!(res.filled_qty, 0);
            assert_eq!(res.resting_qty, 10);
        }
        assert_eq!(client.get_asks().len(), 25);

        // Place 25 crossed bids of 10 units each at price 105, plus 1 larger bid with partial remainder
        for _ in 0..24 {
            let res = client.place_order(&buyer, &Side::Bid, &105, &10, &50);
            assert_eq!(res.filled_qty, 10);
            assert_eq!(res.resting_qty, 0);
        }
        // 25th bid requests 15 units: clears the last 10 ask units and leaves 5 resting in bid queue
        let final_cross = client.place_order(&buyer, &Side::Bid, &105, &15, &50);
        assert_eq!(final_cross.filled_qty, 10);
        assert_eq!(final_cross.resting_qty, 5);

        // All 25 filled asks removed from storage; 1 partial bid remains with 5 units
        assert_eq!(client.get_asks().len(), 0);
        let bids = client.get_bids();
        assert_eq!(bids.len(), 1);
        assert_eq!(bids.get(0).unwrap().remaining_qty, 5);
    }
}

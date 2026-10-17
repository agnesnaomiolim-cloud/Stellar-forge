use crate::queue::{
    insert_ask_sorted, insert_bid_sorted, load_asks, load_bids, save_asks, save_bids, LimitOrder,
    OrderbookError, Side, StorageKey,
};
use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchExecutionSummary {
    pub order_id: u64,
    pub filled_qty: i128,
    pub resting_qty: i128,
    pub matched_orders_count: u32,
}

pub fn submit_order(
    env: &Env,
    maker: Address,
    side: Side,
    price: i128,
    qty: i128,
    max_matches: u32,
) -> Result<MatchExecutionSummary, OrderbookError> {
    if price <= 0 || qty <= 0 || max_matches == 0 {
        return Err(OrderbookError::InvalidOrderParameters);
    }

    let order_id: u64 = env
        .storage()
        .instance()
        .get(&StorageKey::NextOrderId)
        .unwrap_or(1);
    env.storage()
        .instance()
        .set(&StorageKey::NextOrderId, &(order_id + 1));

    let mut remaining_qty = qty;
    let mut filled_qty = 0i128;
    let mut matched_orders_count = 0u32;

    match side {
        Side::Bid => {
            let asks = load_asks(env);
            let mut updated_asks = Vec::new(env);
            for mut ask in asks.iter() {
                if remaining_qty > 0 && matched_orders_count < max_matches && price >= ask.price {
                    let trade_qty = remaining_qty.min(ask.remaining_qty);
                    remaining_qty -= trade_qty;
                    ask.remaining_qty -= trade_qty;
                    filled_qty += trade_qty;
                    matched_orders_count += 1;
                }
                if ask.remaining_qty > 0 {
                    updated_asks.push_back(ask);
                }
            }
            save_asks(env, &updated_asks);

            if remaining_qty > 0 {
                let bids = load_bids(env);
                let next_bids = insert_bid_sorted(
                    env,
                    &bids,
                    LimitOrder {
                        order_id,
                        maker,
                        side: Side::Bid,
                        price,
                        remaining_qty,
                    },
                );
                save_bids(env, &next_bids);
            }
        }
        Side::Ask => {
            let bids = load_bids(env);
            let mut updated_bids = Vec::new(env);
            for mut bid in bids.iter() {
                if remaining_qty > 0 && matched_orders_count < max_matches && price <= bid.price {
                    let trade_qty = remaining_qty.min(bid.remaining_qty);
                    remaining_qty -= trade_qty;
                    bid.remaining_qty -= trade_qty;
                    filled_qty += trade_qty;
                    matched_orders_count += 1;
                }
                if bid.remaining_qty > 0 {
                    updated_bids.push_back(bid);
                }
            }
            save_bids(env, &updated_bids);

            if remaining_qty > 0 {
                let asks = load_asks(env);
                let next_asks = insert_ask_sorted(
                    env,
                    &asks,
                    LimitOrder {
                        order_id,
                        maker,
                        side: Side::Ask,
                        price,
                        remaining_qty,
                    },
                );
                save_asks(env, &next_asks);
            }
        }
    }

    Ok(MatchExecutionSummary {
        order_id,
        filled_qty,
        resting_qty: remaining_qty,
        matched_orders_count,
    })
}

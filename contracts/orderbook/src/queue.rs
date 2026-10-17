use soroban_sdk::{contracterror, contracttype, Address, Env, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OrderbookError {
    InvalidOrderParameters = 1,
    OrderNotFound = 2,
    Unauthorized = 3,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LimitOrder {
    pub order_id: u64,
    pub maker: Address,
    pub side: Side,
    pub price: i128,
    pub remaining_qty: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    NextOrderId,
    AskQueue,
    BidQueue,
}

pub fn load_asks(env: &Env) -> Vec<LimitOrder> {
    env.storage()
        .persistent()
        .get(&StorageKey::AskQueue)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn save_asks(env: &Env, asks: &Vec<LimitOrder>) {
    if asks.is_empty() {
        env.storage().persistent().remove(&StorageKey::AskQueue);
    } else {
        env.storage().persistent().set(&StorageKey::AskQueue, asks);
    }
}

pub fn load_bids(env: &Env) -> Vec<LimitOrder> {
    env.storage()
        .persistent()
        .get(&StorageKey::BidQueue)
        .unwrap_or_else(|| Vec::new(env))
}

pub fn save_bids(env: &Env, bids: &Vec<LimitOrder>) {
    if bids.is_empty() {
        env.storage().persistent().remove(&StorageKey::BidQueue);
    } else {
        env.storage().persistent().set(&StorageKey::BidQueue, bids);
    }
}

/// Inserts an ask maintaining ascending price-time priority.
pub fn insert_ask_sorted(
    env: &Env,
    asks: &Vec<LimitOrder>,
    new_order: LimitOrder,
) -> Vec<LimitOrder> {
    let mut out = Vec::new(env);
    let mut inserted = false;
    for item in asks.iter() {
        if !inserted && new_order.price < item.price {
            out.push_back(new_order.clone());
            inserted = true;
        }
        out.push_back(item);
    }
    if !inserted {
        out.push_back(new_order);
    }
    out
}

/// Inserts a bid maintaining descending price-time priority.
pub fn insert_bid_sorted(
    env: &Env,
    bids: &Vec<LimitOrder>,
    new_order: LimitOrder,
) -> Vec<LimitOrder> {
    let mut out = Vec::new(env);
    let mut inserted = false;
    for item in bids.iter() {
        if !inserted && new_order.price > item.price {
            out.push_back(new_order.clone());
            inserted = true;
        }
        out.push_back(item);
    }
    if !inserted {
        out.push_back(new_order);
    }
    out
}

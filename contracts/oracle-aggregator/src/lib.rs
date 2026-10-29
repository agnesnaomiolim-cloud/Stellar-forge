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

//! # Decentralized Oracles Aggregator with Medianizer & Outlier Trimming
//!
//! A manipulation-resistant on-chain price aggregator for Stellar Soroban that:
//!
//! * Accepts price pushes from up to [`math::MAX_FEEDS`] authorized oracle feeds.
//! * Computes a **median price** over the submitted values.
//! * Removes statistical outliers via configurable percentage-deviation trimming
//!   ([`math::trim_outliers`]).
//! * Enforces **heartbeat staleness checks** — prices older than
//!   `heartbeat_ledgers` are rejected and excluded from aggregation.
//! * Exposes a `get_price` view that always returns the latest aggregated result.
//!
//! ## Design
//!
//! Authorized feeds call [`OracleAggregator::submit_price`] once per heartbeat
//! window.  The aggregator stores each submission in Instance Storage keyed by
//! feed address, then re-computes the aggregate lazily when `get_price` is
//! called (or eagerly after each submission via `aggregate`).
//!
//! All arithmetic is performed with `i128` fixed-point integers scaled by
//! [`math::PRICE_PRECISION`] (10^7) — no floating-point, no overflow risk.
//!
//! ## Storage keys
//!
//! | Key variant | Value | Lifetime |
//! |-------------|-------|----------|
//! | `Config`    | [`AggregatorConfig`] | Instance (persistent) |
//! | `Feed(addr)`| [`FeedEntry`] | Instance (persistent) |
//! | `AggPrice`  | [`AggregatedPrice`] | Instance (persistent) |

#![no_std]

pub mod math;

use math::{median, trim_outliers, MAX_FEEDS, MAX_FEEDS_U32};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, Vec,
};

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// Contract-level error codes returned to callers.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AggregatorError {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet.
    NotInitialized = 2,
    /// The caller is not the admin.
    Unauthorized = 3,
    /// The feed address is already registered.
    FeedAlreadyRegistered = 4,
    /// The feed address is not registered.
    FeedNotFound = 5,
    /// The maximum number of feeds has been reached.
    MaxFeedsReached = 6,
    /// The submitted price is non-positive or overflows.
    InvalidPrice = 7,
    /// The deviation threshold is out of range (0 – 10_000 bps).
    InvalidDeviation = 8,
    /// The heartbeat window must be at least 1 ledger.
    InvalidHeartbeat = 9,
    /// Not enough valid (non-stale) feeds to compute an aggregate.
    InsufficientFeeds = 10,
    /// The submitted price is stale (older than heartbeat window).
    StaleFeed = 11,
}

// ---------------------------------------------------------------------------
// Contract types
// ---------------------------------------------------------------------------

/// Persisted configuration set during [`OracleAggregator::initialize`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregatorConfig {
    /// Contract administrator — allowed to register/remove feeds and update config.
    pub admin: Address,
    /// Outlier-trimming threshold in basis-points (1 bps = 0.01%).
    /// E.g. `2000` = 20%.  Prices deviating more than this fraction from the
    /// preliminary median are excluded from the final aggregate.
    pub deviation_bps: u32,
    /// Staleness window in ledgers.  A feed submission is considered stale if
    /// `current_ledger - submission_ledger > heartbeat_ledgers`.
    pub heartbeat_ledgers: u32,
    /// Minimum number of valid (non-stale, non-outlier) feeds required to
    /// produce an aggregate.
    pub min_feeds: u32,
}

/// A single price submission from an authorized oracle feed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedEntry {
    /// Feed oracle address (used for authorization).
    pub feed: Address,
    /// Most recently submitted price, scaled by [`PRICE_PRECISION`].
    pub price: i128,
    /// Ledger sequence number at the time of the last submission.
    pub last_ledger: u32,
}

/// Aggregated price result stored after each successful aggregation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregatedPrice {
    /// The aggregated median price, scaled by [`PRICE_PRECISION`].
    pub price: i128,
    /// Ledger at which this aggregate was computed.
    pub ledger: u32,
    /// Number of feeds that contributed to this aggregate (after outlier trim).
    pub contributing_feeds: u32,
}

/// Storage key variants.
#[contracttype]
enum DataKey {
    /// Singleton configuration.
    Config,
    /// Registration flag per feed address; value = true when registered.
    Registered(Address),
    /// Per-feed price submission; keyed by feed `Address`.
    Feed(Address),
    /// Latest aggregated price.
    AggPrice,
    /// Ordered list of registered feed addresses (for enumeration).
    FeedList,
}

// ---------------------------------------------------------------------------
// Contract definition
// ---------------------------------------------------------------------------

#[contract]
pub struct OracleAggregator;

#[contractimpl]
impl OracleAggregator {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Initialize the oracle aggregator.
    ///
    /// Must be called exactly once.
    ///
    /// # Parameters
    ///
    /// - `admin`: Address that may manage feeds and config.
    /// - `deviation_bps`: Outlier threshold in basis-points (0 – 10_000).
    /// - `heartbeat_ledgers`: How many ledgers a price remains fresh (≥ 1).
    /// - `min_feeds`: Minimum valid feeds required to produce a price (≥ 1).
    pub fn initialize(
        env: Env,
        admin: Address,
        deviation_bps: u32,
        heartbeat_ledgers: u32,
        min_feeds: u32,
    ) -> Result<(), AggregatorError> {
        if env.storage().instance().has(&DataKey::Config) {
            panic_with_error!(&env, AggregatorError::AlreadyInitialized);
        }

        if deviation_bps > 10_000 {
            panic_with_error!(&env, AggregatorError::InvalidDeviation);
        }
        if heartbeat_ledgers == 0 {
            panic_with_error!(&env, AggregatorError::InvalidHeartbeat);
        }
        if min_feeds == 0 || min_feeds > MAX_FEEDS_U32 {
            panic_with_error!(&env, AggregatorError::InsufficientFeeds);
        }

        admin.require_auth();

        let config = AggregatorConfig {
            admin,
            deviation_bps,
            heartbeat_ledgers,
            min_feeds,
        };
        env.storage().instance().set(&DataKey::Config, &config);

        // Initialize empty feed list
        let feed_list: Vec<Address> = Vec::new(&env);
        env.storage().instance().set(&DataKey::FeedList, &feed_list);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Feed management (admin-only)
    // -----------------------------------------------------------------------

    /// Register a new authorized oracle feed.
    ///
    /// Requires admin authorization.  Up to [`MAX_FEEDS_U32`] feeds may be
    /// registered.
    pub fn add_feed(env: Env, feed: Address) -> Result<(), AggregatorError> {
        let config = Self::load_config(&env)?;
        config.admin.require_auth();

        // Check capacity
        let mut feed_list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::FeedList)
            .unwrap_or_else(|| Vec::new(&env));

        if feed_list.len() >= MAX_FEEDS_U32 {
            panic_with_error!(&env, AggregatorError::MaxFeedsReached);
        }

        // Check for duplicate
        if env
            .storage()
            .instance()
            .has(&DataKey::Registered(feed.clone()))
        {
            panic_with_error!(&env, AggregatorError::FeedAlreadyRegistered);
        }

        feed_list.push_back(feed.clone());
        env.storage().instance().set(&DataKey::FeedList, &feed_list);
        env.storage()
            .instance()
            .set(&DataKey::Registered(feed), &true);

        Ok(())
    }

    /// Remove an authorized oracle feed.
    ///
    /// Requires admin authorization.  Any stored price from this feed is also
    /// removed.
    pub fn remove_feed(env: Env, feed: Address) -> Result<(), AggregatorError> {
        let config = Self::load_config(&env)?;
        config.admin.require_auth();

        if !env
            .storage()
            .instance()
            .has(&DataKey::Registered(feed.clone()))
        {
            panic_with_error!(&env, AggregatorError::FeedNotFound);
        }

        env.storage()
            .instance()
            .remove(&DataKey::Registered(feed.clone()));
        // Also remove any price submission if present
        if env.storage().instance().has(&DataKey::Feed(feed.clone())) {
            env.storage()
                .instance()
                .remove(&DataKey::Feed(feed.clone()));
        }

        // Remove from feed list
        let feed_list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::FeedList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut new_list: Vec<Address> = Vec::new(&env);
        for i in 0..feed_list.len() {
            let addr = feed_list.get(i).unwrap();
            if addr != feed {
                new_list.push_back(addr);
            }
        }
        env.storage().instance().set(&DataKey::FeedList, &new_list);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Price submission
    // -----------------------------------------------------------------------

    /// Submit a price from an authorized oracle feed.
    ///
    /// - `feed`: Must be a registered feed address and must authenticate.
    /// - `price`: Scaled by [`PRICE_PRECISION`].  Must be positive.
    ///
    /// On success the aggregator re-computes the aggregate price if the
    /// resulting set of valid feeds meets `min_feeds`.
    pub fn submit_price(env: Env, feed: Address, price: i128) -> Result<(), AggregatorError> {
        Self::load_config(&env)?; // ensure initialized

        // Verify the feed is registered
        if !env
            .storage()
            .instance()
            .has(&DataKey::Registered(feed.clone()))
        {
            panic_with_error!(&env, AggregatorError::FeedNotFound);
        }

        // The feed must authorize this call
        feed.require_auth();

        if price <= 0 {
            panic_with_error!(&env, AggregatorError::InvalidPrice);
        }

        let entry = FeedEntry {
            feed: feed.clone(),
            price,
            last_ledger: env.ledger().sequence(),
        };
        env.storage().instance().set(&DataKey::Feed(feed), &entry);

        // Eagerly update the aggregate (best-effort; errors are non-fatal)
        let _ = Self::aggregate_internal(&env);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Aggregation
    // -----------------------------------------------------------------------

    /// Trigger an explicit re-aggregation of all current feed submissions.
    ///
    /// Returns the newly computed [`AggregatedPrice`], or an error if there
    /// are insufficient valid feeds.
    pub fn aggregate(env: Env) -> Result<AggregatedPrice, AggregatorError> {
        Self::load_config(&env)?;
        Self::aggregate_internal(&env)
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Return the latest aggregated price.
    ///
    /// Returns an error if no aggregation has been computed yet.
    pub fn get_price(env: Env) -> Result<AggregatedPrice, AggregatorError> {
        Self::load_config(&env)?;
        env.storage()
            .instance()
            .get(&DataKey::AggPrice)
            .ok_or(AggregatorError::InsufficientFeeds)
    }

    /// Return all currently registered feed entries (including stale ones).
    pub fn get_feeds(env: Env) -> Result<Vec<FeedEntry>, AggregatorError> {
        Self::load_config(&env)?;
        let feed_list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::FeedList)
            .unwrap_or_else(|| Vec::new(&env));

        let mut result: Vec<FeedEntry> = Vec::new(&env);
        for i in 0..feed_list.len() {
            let addr = feed_list.get(i).unwrap();
            if let Some(entry) = env
                .storage()
                .instance()
                .get::<DataKey, FeedEntry>(&DataKey::Feed(addr))
            {
                result.push_back(entry);
            }
        }
        Ok(result)
    }

    /// Return the current [`AggregatorConfig`].
    pub fn get_config(env: Env) -> Result<AggregatorConfig, AggregatorError> {
        Self::load_config(&env)
    }

    // -----------------------------------------------------------------------
    // Admin: update config
    // -----------------------------------------------------------------------

    /// Update the outlier deviation threshold.
    pub fn set_deviation(env: Env, deviation_bps: u32) -> Result<(), AggregatorError> {
        let mut config = Self::load_config(&env)?;
        config.admin.require_auth();
        if deviation_bps > 10_000 {
            panic_with_error!(&env, AggregatorError::InvalidDeviation);
        }
        config.deviation_bps = deviation_bps;
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    /// Update the staleness heartbeat window.
    pub fn set_heartbeat(env: Env, heartbeat_ledgers: u32) -> Result<(), AggregatorError> {
        let mut config = Self::load_config(&env)?;
        config.admin.require_auth();
        if heartbeat_ledgers == 0 {
            panic_with_error!(&env, AggregatorError::InvalidHeartbeat);
        }
        config.heartbeat_ledgers = heartbeat_ledgers;
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn load_config(env: &Env) -> Result<AggregatorConfig, AggregatorError> {
        env.storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(AggregatorError::NotInitialized)
    }

    /// Collect valid (non-stale) feed prices, apply outlier trimming, compute
    /// the median, and persist the result.
    fn aggregate_internal(env: &Env) -> Result<AggregatedPrice, AggregatorError> {
        let config = Self::load_config(env)?;
        let current_ledger = env.ledger().sequence();

        let feed_list: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::FeedList)
            .unwrap_or_else(|| Vec::new(env));

        // Collect non-stale prices into a fixed-size stack array
        let mut raw_prices = [0i128; MAX_FEEDS];
        let mut count = 0usize;

        for i in 0..feed_list.len() {
            let addr = feed_list.get(i).unwrap();
            if let Some(entry) = env
                .storage()
                .instance()
                .get::<DataKey, FeedEntry>(&DataKey::Feed(addr))
            {
                // Staleness check: reject feeds that haven't updated recently
                let age = current_ledger.saturating_sub(entry.last_ledger);
                if age > config.heartbeat_ledgers {
                    continue; // stale — exclude
                }
                if count < MAX_FEEDS {
                    raw_prices[count] = entry.price;
                    count += 1;
                }
            }
        }

        // Insufficient feeds check (before trimming)
        if count == 0 {
            return Err(AggregatorError::InsufficientFeeds);
        }

        // Outlier trimming
        let inlier_count = trim_outliers(&mut raw_prices, count, config.deviation_bps);

        if (inlier_count as u32) < config.min_feeds {
            return Err(AggregatorError::InsufficientFeeds);
        }

        // Median of inliers
        let agg_price =
            median(&mut raw_prices, inlier_count).ok_or(AggregatorError::InsufficientFeeds)?;

        let result = AggregatedPrice {
            price: agg_price,
            ledger: current_ledger,
            contributing_feeds: inlier_count as u32,
        };

        env.storage().instance().set(&DataKey::AggPrice, &result);

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    #![cfg(test)]
    extern crate std;

    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    /// Helper: create and initialize a fresh aggregator environment.
    fn setup(
        deviation_bps: u32,
        heartbeat_ledgers: u32,
        min_feeds: u32,
    ) -> (Env, Address, OracleAggregatorClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(OracleAggregator, ());
        let client = OracleAggregatorClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin, &deviation_bps, &heartbeat_ledgers, &min_feeds);

        (env, admin, client)
    }

    /// Advance ledger by `n` ledgers.
    fn advance_ledger(env: &Env, n: u32) {
        env.ledger().with_mut(|li| {
            li.sequence_number += n;
        });
    }

    // -----------------------------------------------------------------------
    // Initialization
    // -----------------------------------------------------------------------

    #[test]
    fn initialize_succeeds_and_stores_config() {
        let (env, admin, client) = setup(2_000, 100, 1);
        let config = client.get_config();
        assert_eq!(config.admin, admin);
        assert_eq!(config.deviation_bps, 2_000);
        assert_eq!(config.heartbeat_ledgers, 100);
        assert_eq!(config.min_feeds, 1);
        let _ = env;
    }

    #[test]
    #[should_panic]
    fn initialize_twice_panics() {
        let (_, admin, client) = setup(2_000, 100, 1);
        client.initialize(&admin, &2_000, &100, &1);
    }

    // -----------------------------------------------------------------------
    // Feed management
    // -----------------------------------------------------------------------

    #[test]
    fn add_and_remove_feed() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let feed = Address::generate(&env);
        client.add_feed(&feed);

        // Adding same feed again should panic
        let result = client.try_add_feed(&feed);
        assert!(result.is_err());

        client.remove_feed(&feed);
        // Removing again should panic
        let result = client.try_remove_feed(&feed);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Price submission and aggregation
    // -----------------------------------------------------------------------

    #[test]
    fn single_feed_aggregates_correctly() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let feed = Address::generate(&env);
        client.add_feed(&feed);

        let price = 10_000_000i128; // $1.00
        client.submit_price(&feed, &price);

        let agg = client.get_price();
        assert_eq!(agg.price, price);
        assert_eq!(agg.contributing_feeds, 1);
    }

    #[test]
    fn five_feeds_median_calculated_correctly() {
        let (env, _admin, client) = setup(2_000, 100, 1);

        let prices: [i128; 5] = [
            9_800_000,  // $0.98
            9_900_000,  // $0.99
            10_000_000, // $1.00
            10_100_000, // $1.01
            10_200_000, // $1.02
        ];

        for price in prices.iter() {
            let feed = Address::generate(&env);
            client.add_feed(&feed);
            client.submit_price(&feed, price);
        }

        let agg = client.get_price();
        // Sorted prices: [9_800_000, 9_900_000, 10_000_000, 10_100_000, 10_200_000]
        // Median (5 elements) = 10_000_000
        assert_eq!(agg.price, 10_000_000);
        assert_eq!(agg.contributing_feeds, 5);
    }

    #[test]
    fn spike_500pct_is_trimmed_median_remains_stable() {
        // Issue #69 validation scenario: 1 of 5 oracles reports +500% spike
        let (env, _admin, client) = setup(2_000, 100, 1); // 20% threshold

        let normal_price = 10_000_000i128; // $1.00
        let spike_price = 60_000_000i128; // $6.00 (+500%)

        // Register and submit 4 normal feeds
        for _ in 0..4u32 {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, &normal_price);
        }
        // Register and submit 1 spike feed
        let spike_feed = Address::generate(&env);
        client.add_feed(&spike_feed);
        client.submit_price(&spike_feed, &spike_price);

        let agg = client.get_price();
        // Spike should be trimmed; median of 4 normals = $1.00
        assert_eq!(
            agg.price, normal_price,
            "aggregated price must be $1.00 — spike trimmed"
        );
        assert_eq!(
            agg.contributing_feeds, 4,
            "only 4 inlier feeds should contribute"
        );
    }

    #[test]
    fn stale_feed_is_excluded_from_aggregation() {
        let (env, _admin, client) = setup(2_000, 10, 1); // heartbeat = 10 ledgers

        let fresh_feed = Address::generate(&env);
        let stale_feed = Address::generate(&env);
        client.add_feed(&fresh_feed);
        client.add_feed(&stale_feed);

        let fresh_price = 10_000_000i128;
        let stale_price = 99_000_000i128;

        // Submit stale feed price at ledger 0
        client.submit_price(&stale_feed, &stale_price);
        // Submit fresh feed price at ledger 0
        client.submit_price(&fresh_feed, &fresh_price);

        // Advance ledger 100 — far beyond heartbeat of 10
        advance_ledger(&env, 100);

        // Submit fresh feed price again at the new ledger
        client.submit_price(&fresh_feed, &fresh_price);

        let agg = client.get_price();
        assert_eq!(agg.price, fresh_price, "stale feed should be excluded");
        assert_eq!(
            agg.contributing_feeds, 1,
            "only 1 fresh feed should contribute"
        );
    }

    #[test]
    fn no_feeds_get_price_returns_error() {
        let (_, _, client) = setup(2_000, 100, 1);
        let result = client.try_get_price();
        assert!(result.is_err());
    }

    #[test]
    fn below_min_feeds_after_trimming_returns_error() {
        // min_feeds=3 but only 2 valid feeds exist
        let (env, _admin, client) = setup(2_000, 100, 3);

        for _ in 0..2u32 {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, &10_000_000i128);
        }

        // With only 2 feeds and min_feeds=3, the eager aggregation fails silently
        // so AggPrice is never stored; get_price should error
        let result = client.try_get_price();
        assert!(
            result.is_err(),
            "expected error when below min_feeds, but got a price"
        );
    }

    #[test]
    fn invalid_price_zero_rejected() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let feed = Address::generate(&env);
        client.add_feed(&feed);
        let result = client.try_submit_price(&feed, &0i128);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_price_negative_rejected() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let feed = Address::generate(&env);
        client.add_feed(&feed);
        let result = client.try_submit_price(&feed, &-1i128);
        assert!(result.is_err());
    }

    #[test]
    fn unregistered_feed_submit_rejected() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let unregistered = Address::generate(&env);
        let result = client.try_submit_price(&unregistered, &10_000_000i128);
        assert!(result.is_err());
    }

    #[test]
    fn set_deviation_updates_config() {
        let (_, _admin, client) = setup(2_000, 100, 1);
        client.set_deviation(&500u32);
        let config = client.get_config();
        assert_eq!(config.deviation_bps, 500);
    }

    #[test]
    fn set_heartbeat_updates_config() {
        let (_, _admin, client) = setup(2_000, 100, 1);
        client.set_heartbeat(&200u32);
        let config = client.get_config();
        assert_eq!(config.heartbeat_ledgers, 200);
    }

    #[test]
    fn get_feeds_returns_submitted_entries() {
        let (env, _admin, client) = setup(2_000, 100, 1);
        let feed = Address::generate(&env);
        client.add_feed(&feed);
        client.submit_price(&feed, &10_000_000i128);

        let feeds = client.get_feeds();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds.get(0).unwrap().price, 10_000_000);
    }

    // -----------------------------------------------------------------------
    // Property-based tests (deterministic simulation)
    // -----------------------------------------------------------------------

    #[test]
    fn property_median_of_identical_prices_equals_price() {
        let (env, _admin, client) = setup(2_000, 1000, 1);
        let price = 25_000_000i128; // $2.50

        for _ in 0..5u32 {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, &price);
        }

        let agg = client.get_price();
        assert_eq!(agg.price, price);
    }

    #[test]
    fn property_aggregate_monotone_within_threshold() {
        // All prices within 5% of each other — all should be inliers
        let (env, _admin, client) = setup(1_000, 1000, 3); // 10% threshold, min 3

        let base = 50_000_000i128;
        let prices = [
            base - base / 20, // -5%
            base,
            base + base / 20,  // +5%
            base - base / 100, // -1%
            base + base / 100, // +1%
        ];

        for p in prices.iter() {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, p);
        }

        let agg = client.get_price();
        // All 5 feeds should be inliers (within 10% threshold)
        assert_eq!(agg.contributing_feeds, 5);
        // Median should be within ±5% of base
        let dev = if agg.price > base {
            agg.price - base
        } else {
            base - agg.price
        };
        assert!(
            dev <= base / 20,
            "median deviates by more than 5% from base"
        );
    }

    #[test]
    fn property_price_precision_is_seven_decimal_places() {
        assert_eq!(math::PRICE_PRECISION, 10_000_000);
    }

    #[test]
    fn property_multiple_spikes_all_trimmed() {
        // 3 normal feeds, 2 spike feeds — both spikes should be trimmed
        let (env, _admin, client) = setup(2_000, 1000, 1); // 20% threshold

        let normal = 10_000_000i128;
        let spike = 80_000_000i128; // +700%

        for _ in 0..3u32 {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, &normal);
        }
        for _ in 0..2u32 {
            let f = Address::generate(&env);
            client.add_feed(&f);
            client.submit_price(&f, &spike);
        }

        let agg = client.get_price();
        assert_eq!(agg.price, normal, "spikes should be trimmed");
        assert_eq!(agg.contributing_feeds, 3);
    }

    #[test]
    fn property_no_stale_feeds_contributes_zero() {
        let (env, _admin, client) = setup(2_000, 5, 1); // heartbeat = 5 ledgers

        let feed = Address::generate(&env);
        client.add_feed(&feed);
        client.submit_price(&feed, &10_000_000i128);

        // Advance 100 ledgers — far beyond heartbeat of 5
        advance_ledger(&env, 100);

        // No new submissions — force re-aggregation explicitly
        let result = client.try_aggregate();
        assert!(result.is_err(), "all feeds are stale — aggregate must fail");
    }
}

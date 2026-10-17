#![no_std]

pub mod math;

use math::{aggregate_with_outlier_trimming, AggregationResult, OracleError};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeedSubmission {
    pub oracle: Address,
    pub price: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregatorConfig {
    pub admin: Address,
    pub min_quorum: u32,
    pub max_staleness_secs: u64,
    pub max_deviation_bps: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum StorageKey {
    Config,
    AuthorizedOracles,
    Submission(Address),
    LatestResult,
}

#[contract]
pub struct OracleAggregatorContract;

#[contractimpl]
impl OracleAggregatorContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        oracles: Vec<Address>,
        min_quorum: u32,
        max_staleness_secs: u64,
        max_deviation_bps: i128,
    ) -> Result<(), OracleError> {
        if env.storage().instance().has(&StorageKey::Config) {
            return Err(OracleError::AlreadyInitialized);
        }
        if min_quorum == 0 || oracles.len() < min_quorum || max_deviation_bps <= 0 {
            return Err(OracleError::InvalidReading);
        }
        admin.require_auth();
        env.storage().instance().set(
            &StorageKey::Config,
            &AggregatorConfig {
                admin,
                min_quorum,
                max_staleness_secs,
                max_deviation_bps,
            },
        );
        env.storage()
            .instance()
            .set(&StorageKey::AuthorizedOracles, &oracles);
        Ok(())
    }

    pub fn push_price(env: Env, oracle: Address, price: i128) -> Result<(), OracleError> {
        oracle.require_auth();
        if price <= 0 {
            return Err(OracleError::InvalidReading);
        }
        let oracles: Vec<Address> = env
            .storage()
            .instance()
            .get(&StorageKey::AuthorizedOracles)
            .ok_or(OracleError::NotInitialized)?;
        if !oracles.contains(&oracle) {
            return Err(OracleError::Unauthorized);
        }

        let sub = FeedSubmission {
            oracle: oracle.clone(),
            price,
            timestamp: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&StorageKey::Submission(oracle), &sub);
        Ok(())
    }

    pub fn compute_aggregated_price(env: Env) -> Result<AggregationResult, OracleError> {
        let config: AggregatorConfig = env
            .storage()
            .instance()
            .get(&StorageKey::Config)
            .ok_or(OracleError::NotInitialized)?;
        let oracles: Vec<Address> = env
            .storage()
            .instance()
            .get(&StorageKey::AuthorizedOracles)
            .ok_or(OracleError::NotInitialized)?;

        let now = env.ledger().timestamp();
        let mut fresh_prices = Vec::new(&env);
        let mut stale_excluded_count = 0u32;

        for oracle in oracles.iter() {
            if let Some(sub) = env
                .storage()
                .persistent()
                .get::<StorageKey, FeedSubmission>(&StorageKey::Submission(oracle))
            {
                if now.saturating_sub(sub.timestamp) <= config.max_staleness_secs {
                    fresh_prices.push_back(sub.price);
                } else {
                    stale_excluded_count += 1;
                }
            }
        }

        let result = aggregate_with_outlier_trimming(
            &env,
            &fresh_prices,
            stale_excluded_count,
            config.min_quorum,
            config.max_deviation_bps,
        )?;
        env.storage()
            .instance()
            .set(&StorageKey::LatestResult, &result);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};

    #[test]
    fn test_500_percent_spike_trimmed_and_stale_feeds_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let contract_id = env.register(OracleAggregatorContract, ());
        let client = OracleAggregatorContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let o1 = Address::generate(&env);
        let o2 = Address::generate(&env);
        let o3 = Address::generate(&env);
        let o4 = Address::generate(&env);
        let o5 = Address::generate(&env);

        let mut oracles = Vec::new(&env);
        oracles.push_back(o1.clone());
        oracles.push_back(o2.clone());
        oracles.push_back(o3.clone());
        oracles.push_back(o4.clone());
        oracles.push_back(o5.clone());

        client.initialize(&admin, &oracles, &3, &300, &1_500); // max 15% deviation

        // o1 pushes early at t=1_000 (will become stale at t=2_000)
        client.push_price(&o1, &100_000);

        env.ledger().set_timestamp(2_000);
        // 3 honest feeds around 100_000 + 1 manipulated feed spiking +500% (600_000)
        client.push_price(&o2, &99_500);
        client.push_price(&o3, &100_000);
        client.push_price(&o4, &100_500);
        client.push_price(&o5, &600_000); // +500% spike

        let res = client.compute_aggregated_price();
        assert_eq!(res.stale_excluded_count, 1);
        assert_eq!(res.trimmed_outlier_count, 1);
        assert_eq!(res.valid_feed_count, 3);
        assert_eq!(res.median_price, 100_000);
    }
}

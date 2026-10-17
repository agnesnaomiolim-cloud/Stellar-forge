use soroban_sdk::{contracterror, contracttype, Env, Vec};

pub const BPS_SCALE: i128 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidReading = 4,
    InsufficientQuorum = 5,
    ArithmeticOverflow = 6,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregationResult {
    pub median_price: i128,
    pub valid_feed_count: u32,
    pub trimmed_outlier_count: u32,
    pub stale_excluded_count: u32,
}

pub fn sort_prices(env: &Env, values: &Vec<i128>) -> Vec<i128> {
    let mut sorted = Vec::new(env);
    for val in values.iter() {
        let mut next = Vec::new(env);
        let mut inserted = false;
        for existing in sorted.iter() {
            if !inserted && val < existing {
                next.push_back(val);
                inserted = true;
            }
            next.push_back(existing);
        }
        if !inserted {
            next.push_back(val);
        }
        sorted = next;
    }
    sorted
}

pub fn compute_median(sorted: &Vec<i128>) -> Result<i128, OracleError> {
    let n = sorted.len();
    if n == 0 {
        return Err(OracleError::InsufficientQuorum);
    }
    let mid = n / 2;
    if n % 2 == 1 {
        Ok(sorted.get(mid).unwrap())
    } else {
        let a = sorted.get(mid - 1).unwrap();
        let b = sorted.get(mid).unwrap();
        let sum = a.checked_add(b).ok_or(OracleError::ArithmeticOverflow)?;
        Ok(sum / 2)
    }
}

pub fn aggregate_with_outlier_trimming(
    env: &Env,
    fresh_prices: &Vec<i128>,
    stale_excluded_count: u32,
    min_quorum: u32,
    max_deviation_bps: i128,
) -> Result<AggregationResult, OracleError> {
    if fresh_prices.len() < min_quorum {
        return Err(OracleError::InsufficientQuorum);
    }

    let sorted_initial = sort_prices(env, fresh_prices);
    let raw_median = compute_median(&sorted_initial)?;

    let mut filtered = Vec::new(env);
    let mut trimmed_outlier_count = 0u32;

    for price in sorted_initial.iter() {
        let diff = (price - raw_median).abs();
        let dev_bps = diff
            .checked_mul(BPS_SCALE)
            .ok_or(OracleError::ArithmeticOverflow)?
            / raw_median.max(1);
        if dev_bps <= max_deviation_bps {
            filtered.push_back(price);
        } else {
            trimmed_outlier_count += 1;
        }
    }

    if filtered.len() < min_quorum {
        return Err(OracleError::InsufficientQuorum);
    }

    let final_median = compute_median(&filtered)?;
    Ok(AggregationResult {
        median_price: final_median,
        valid_feed_count: filtered.len(),
        trimmed_outlier_count,
        stale_excluded_count,
    })
}

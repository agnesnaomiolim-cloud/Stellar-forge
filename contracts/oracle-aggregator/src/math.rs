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

//! Overflow-safe integer arithmetic for oracle price aggregation.
//!
//! All calculations use fixed-point integer arithmetic with a configurable
//! precision scalar (`PRICE_PRECISION`).  Floating-point arithmetic is never
//! used — the `#![no_std]` environment does not support it, and integer math
//! avoids the rounding non-determinism that would break cross-node consensus.
//!
//! # Precision
//!
//! Prices are represented as `i128` scaled by `PRICE_PRECISION` (10^7).
//! A price of `1.5000000` is stored as `15_000_000_i128`.
//!
//! # Outlier trimming
//!
//! Given N submitted prices and a `deviation_bps` threshold (in basis-points,
//! i.e. 1 bps = 0.01%), any price whose absolute deviation from the initial
//! median exceeds the threshold is dropped.  The final aggregate is computed
//! as the median of the remaining (inlier) prices.
//!
//! # Sorting
//!
//! Because `#![no_std]` disallows dynamic allocation via `std`, all sorting
//! is performed in-place on a fixed-size array on the stack.  The maximum
//! number of oracle feeds (`MAX_FEEDS = 20`) is small enough that an
//! insertion-sort or selection-sort is both correct and gas-efficient.

/// Decimal precision scalar: 7 decimal places (1 = 0.0000001).
pub const PRICE_PRECISION: i128 = 10_000_000; // 10^7

/// Maximum number of oracle feeds the aggregator tracks.
pub const MAX_FEEDS: usize = 20;

/// Maximum number of feeds that may be registered by the admin.
pub const MAX_FEEDS_U32: u32 = MAX_FEEDS as u32;

/// Basis-points denominator (10_000 bps = 100%).
pub const BPS_DENOM: i128 = 10_000;

// ---------------------------------------------------------------------------
// Sorting helpers (no-std, fixed-size array)
// ---------------------------------------------------------------------------

/// In-place insertion sort for a slice of `i128`.
///
/// O(n²) but n ≤ 20, so the gas cost is negligible.
pub fn sort_prices(prices: &mut [i128]) {
    let n = prices.len();
    for i in 1..n {
        let key = prices[i];
        let mut j = i;
        while j > 0 && prices[j - 1] > key {
            prices[j] = prices[j - 1];
            j -= 1;
        }
        prices[j] = key;
    }
}

// ---------------------------------------------------------------------------
// Median
// ---------------------------------------------------------------------------

/// Compute the median of a **sorted** price slice.
///
/// Returns `None` if the slice is empty.
///
/// For an even-length slice the median is the average of the two middle
/// values, computed with integer arithmetic:
/// `(a + b) / 2` — both `a` and `b` are bounded by `PRICE_PRECISION`-scaled
/// prices, so the sum fits comfortably inside `i128`.
pub fn median_sorted(sorted: &[i128]) -> Option<i128> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    if n % 2 == 1 {
        Some(sorted[n / 2])
    } else {
        let mid = n / 2;
        // Both values are scaled prices; sum fits in i128 because each price
        // is at most i128::MAX / 2 in practice.
        Some((sorted[mid - 1] + sorted[mid]) / 2)
    }
}

/// Sort `prices[..count]` in-place and return the median.
///
/// `count` must be ≤ `MAX_FEEDS`.
pub fn median(prices: &mut [i128], count: usize) -> Option<i128> {
    if count == 0 || count > prices.len() {
        return None;
    }
    sort_prices(&mut prices[..count]);
    median_sorted(&prices[..count])
}

// ---------------------------------------------------------------------------
// Outlier trimming
// ---------------------------------------------------------------------------

/// Absolute deviation of `price` from `reference`, in the same scale.
///
/// Uses saturating subtraction to avoid overflow on pathological inputs.
#[inline]
pub fn abs_deviation(price: i128, reference: i128) -> i128 {
    if price >= reference {
        price.saturating_sub(reference)
    } else {
        reference.saturating_sub(price)
    }
}

/// Maximum allowed deviation for a price to be treated as an inlier.
///
/// `reference * deviation_bps / BPS_DENOM` — computed with `i128` to
/// prevent overflow.  Both operands fit easily: `reference` is a
/// `PRICE_PRECISION`-scaled price and `deviation_bps` ≤ 10_000.
pub fn max_allowed_deviation(reference: i128, deviation_bps: u32) -> i128 {
    // reference * deviation_bps may overflow if reference is very large;
    // we use checked_mul and fall back to i128::MAX so outliers are still
    // detected when prices saturate.
    match reference.checked_mul(deviation_bps as i128) {
        Some(product) => product / BPS_DENOM,
        None => i128::MAX,
    }
}

/// Filter `prices[..count]` in-place, retaining only inlier prices.
///
/// Returns the number of inliers written to the beginning of `prices`.
///
/// Algorithm:
/// 1. Compute the **preliminary median** of all submitted prices.
/// 2. Discard any price whose absolute deviation from the preliminary median
///    exceeds `deviation_bps / 10_000` of the median.
/// 3. Return the count of surviving inliers.
///
/// If the preliminary median cannot be computed (zero feeds) the function
/// returns 0.
///
/// # Spike simulation note
///
/// When 1 of 5 oracles reports a +500% spike, its price is 6× the median.
/// With `deviation_bps = 2000` (20% threshold), the deviation is 500% of the
/// median, which exceeds 20%, so the spike is trimmed.
pub fn trim_outliers(prices: &mut [i128], count: usize, deviation_bps: u32) -> usize {
    if count == 0 {
        return 0;
    }

    // Step 1: copy to a temporary buffer and sort to find the preliminary median.
    // We keep the original `prices` untouched for now.
    let mut tmp = [0i128; MAX_FEEDS];
    let n = count.min(MAX_FEEDS);
    tmp[..n].copy_from_slice(&prices[..n]);
    sort_prices(&mut tmp[..n]);

    let prelim_median = match median_sorted(&tmp[..n]) {
        Some(m) => m,
        None => return 0,
    };

    // A median of 0 would make every price appear to be an outlier.
    // When the reference is 0 we skip outlier trimming.
    if prelim_median == 0 {
        return n;
    }

    let max_dev = max_allowed_deviation(prelim_median.abs(), deviation_bps);

    // Step 2: compact inliers to the front of the original `prices` slice.
    let mut inlier_count = 0usize;
    for i in 0..n {
        let dev = abs_deviation(prices[i], prelim_median);
        if dev <= max_dev {
            prices[inlier_count] = prices[i];
            inlier_count += 1;
        }
    }

    inlier_count
}

// ---------------------------------------------------------------------------
// Precision helpers
// ---------------------------------------------------------------------------

/// Convert a raw integer price to `PRICE_PRECISION`-scaled representation.
///
/// `raw_price * PRICE_PRECISION` — safe via `checked_mul`; returns `None`
/// on overflow.
#[allow(dead_code)]
pub fn scale_price(raw_price: i64) -> Option<i128> {
    (raw_price as i128).checked_mul(PRICE_PRECISION)
}

/// Convert a `PRICE_PRECISION`-scaled price back to integer units.
#[allow(dead_code)]
pub fn unscale_price(scaled: i128) -> i128 {
    scaled / PRICE_PRECISION
}

// ---------------------------------------------------------------------------
// Tests (run with `cargo test` from the contract directory)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // sort_prices
    // ------------------------------------------------------------------

    #[test]
    fn sort_empty_slice_is_no_op() {
        let mut v: [i128; 0] = [];
        sort_prices(&mut v);
    }

    #[test]
    fn sort_single_element() {
        let mut v = [42i128];
        sort_prices(&mut v);
        assert_eq!(v[0], 42);
    }

    #[test]
    fn sort_already_sorted() {
        let mut v = [1i128, 2, 3, 4, 5];
        sort_prices(&mut v);
        assert_eq!(v, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn sort_reverse_sorted() {
        let mut v = [5i128, 4, 3, 2, 1];
        sort_prices(&mut v);
        assert_eq!(v, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn sort_with_duplicates() {
        let mut v = [3i128, 1, 4, 1, 5, 9, 2, 6, 5, 3];
        sort_prices(&mut v);
        assert_eq!(v, [1, 1, 2, 3, 3, 4, 5, 5, 6, 9]);
    }

    // ------------------------------------------------------------------
    // median_sorted
    // ------------------------------------------------------------------

    #[test]
    fn median_empty_returns_none() {
        assert_eq!(median_sorted(&[]), None);
    }

    #[test]
    fn median_one_element() {
        assert_eq!(median_sorted(&[7]), Some(7));
    }

    #[test]
    fn median_odd_count() {
        // [1, 2, 3, 4, 5] → middle = 3
        assert_eq!(median_sorted(&[1, 2, 3, 4, 5]), Some(3));
    }

    #[test]
    fn median_even_count() {
        // [1, 2, 3, 4] → (2 + 3) / 2 = 2
        assert_eq!(median_sorted(&[1, 2, 3, 4]), Some(2));
    }

    #[test]
    fn median_two_elements() {
        assert_eq!(median_sorted(&[10, 20]), Some(15));
    }

    #[test]
    fn median_precision_scaled_five_feeds() {
        // Five oracle prices, all scaled: $1.00 = 10_000_000
        let price = PRICE_PRECISION; // 1.00
        let mut prices = [price, price, price, price, price];
        sort_prices(&mut prices);
        assert_eq!(median_sorted(&prices), Some(price));
    }

    #[test]
    fn median_precision_scaled_even_feeds() {
        // Four oracle prices: 1.0, 1.2, 1.4, 1.6
        let mut prices = [
            12_000_000i128,
            16_000_000i128,
            10_000_000i128,
            14_000_000i128,
        ];
        sort_prices(&mut prices);
        // sorted: [10_000_000, 12_000_000, 14_000_000, 16_000_000]
        // median: (12_000_000 + 14_000_000) / 2 = 13_000_000
        assert_eq!(median_sorted(&prices), Some(13_000_000));
    }

    // ------------------------------------------------------------------
    // abs_deviation
    // ------------------------------------------------------------------

    #[test]
    fn abs_deviation_positive() {
        assert_eq!(abs_deviation(15, 10), 5);
    }

    #[test]
    fn abs_deviation_negative() {
        assert_eq!(abs_deviation(5, 10), 5);
    }

    #[test]
    fn abs_deviation_equal() {
        assert_eq!(abs_deviation(7, 7), 0);
    }

    // ------------------------------------------------------------------
    // max_allowed_deviation
    // ------------------------------------------------------------------

    #[test]
    fn max_allowed_deviation_20pct() {
        // 20% of 10_000_000 = 2_000_000
        let result = max_allowed_deviation(10_000_000, 2_000);
        assert_eq!(result, 2_000_000);
    }

    #[test]
    fn max_allowed_deviation_100pct() {
        let result = max_allowed_deviation(10_000_000, 10_000);
        assert_eq!(result, 10_000_000);
    }

    #[test]
    fn max_allowed_deviation_zero_bps() {
        let result = max_allowed_deviation(10_000_000, 0);
        assert_eq!(result, 0);
    }

    // ------------------------------------------------------------------
    // trim_outliers — core correctness
    // ------------------------------------------------------------------

    #[test]
    fn trim_no_outliers_keeps_all() {
        // Five feeds near $1.00 — no outliers
        let mut prices = [
            10_000_000i128, // 1.00
            10_100_000i128, // 1.01
            9_900_000i128,  // 0.99
            10_050_000i128, // 1.005
            9_950_000i128,  // 0.995
        ];
        let count = trim_outliers(&mut prices, 5, 2_000); // 20% threshold
        assert_eq!(count, 5);
    }

    #[test]
    fn trim_spike_one_of_five_plus_500pct() {
        // Validation scenario from issue #69:
        // 4 oracles at ~$1.00, 1 oracle at +500% ($6.00)
        let normal = 10_000_000i128; // $1.00
        let spike = 60_000_000i128; // $6.00 (+500%)
        let mut prices = [normal, normal, normal, normal, spike];

        // Use 20% (2000 bps) threshold
        let inliers = trim_outliers(&mut prices, 5, 2_000);

        // Spike should be removed; 4 normals remain
        assert_eq!(inliers, 4, "spike should be trimmed");

        // Verify remaining prices are all $1.00
        for i in 0..inliers {
            assert_eq!(
                prices[i], normal,
                "inlier at index {} should be normal price",
                i
            );
        }
    }

    #[test]
    fn trim_spike_median_remains_stable() {
        // After trimming the 500% spike, the median should remain at $1.00
        let normal = 10_000_000i128;
        let spike = 60_000_000i128;
        let mut prices = [normal, normal, normal, normal, spike];
        let inliers = trim_outliers(&mut prices, 5, 2_000);

        let med = median(&mut prices, inliers).unwrap();
        assert_eq!(
            med, normal,
            "median must remain stable after spike trimming"
        );
    }

    #[test]
    fn trim_all_outliers_returns_zero() {
        // Use 0 bps → only exact matches to median survive
        let mut prices = [10i128, 20, 30, 40, 50];
        // median = 30; max_dev = 0 → only 30 passes
        let inliers = trim_outliers(&mut prices, 5, 0);
        assert_eq!(inliers, 1);
        assert_eq!(prices[0], 30);
    }

    #[test]
    fn trim_empty_count_returns_zero() {
        let mut prices = [1i128, 2, 3];
        let count = trim_outliers(&mut prices, 0, 2_000);
        assert_eq!(count, 0);
    }

    #[test]
    fn trim_single_feed_always_inlier() {
        let mut prices = [99_000_000i128]; // $9.90
        let count = trim_outliers(&mut prices, 1, 0); // 0% threshold
                                                      // With 1 feed, median = that feed, deviation = 0 → always inlier
        assert_eq!(count, 1);
    }

    // ------------------------------------------------------------------
    // Property: median is stable under outlier trimming
    // ------------------------------------------------------------------

    #[test]
    fn property_median_monotone_within_inlier_range() {
        // If all prices are within ±10% of the median, trimming changes nothing
        let base = 50_000_000i128; // $5.00
        let ten_pct = base / 10;
        let mut prices = [
            base - ten_pct + 1,
            base - ten_pct / 2,
            base,
            base + ten_pct / 2,
            base + ten_pct - 1,
        ];
        let original_count = 5;
        // 1000 bps = 10% threshold
        let inliers = trim_outliers(&mut prices, original_count, 1_000);
        assert_eq!(inliers, original_count, "no prices should be trimmed");
    }

    #[test]
    fn property_no_overflow_with_large_prices() {
        // Prices near i64::MAX scaled — ensure no overflow in deviation calc
        let large = i64::MAX as i128; // very large price
        let mut prices = [large, large, large, large, large];
        let inliers = trim_outliers(&mut prices, 5, 2_000);
        assert_eq!(inliers, 5); // all identical → all inliers
    }

    #[test]
    fn property_odd_count_median_is_exact_middle() {
        let mut arr = [3i128, 1, 4, 1, 5]; // sorted: [1,1,3,4,5] → median = 3
        let m = median(&mut arr, 5).unwrap();
        assert_eq!(m, 3);
    }

    #[test]
    fn property_even_count_median_is_average_of_middles() {
        let mut arr = [4i128, 1, 3, 2]; // sorted: [1,2,3,4] → median = 2
        let m = median(&mut arr, 4).unwrap();
        assert_eq!(m, 2);
    }

    #[test]
    fn property_scale_unscale_round_trip() {
        let raw = 42i64;
        let scaled = scale_price(raw).unwrap();
        let back = unscale_price(scaled);
        assert_eq!(back, raw as i128);
    }

    #[test]
    fn property_trim_then_median_stable_under_one_spike() {
        // Parametric: vary the number of normal feeds (3..=10) with one spike
        for normal_count in 3usize..=10 {
            let normal_price = 20_000_000i128; // $2.00
            let spike_price = normal_price * 7; // +600%

            let mut prices = [0i128; MAX_FEEDS];
            for i in 0..normal_count {
                prices[i] = normal_price;
            }
            prices[normal_count] = spike_price;

            let total = normal_count + 1;
            let inliers = trim_outliers(&mut prices, total, 1_000); // 10% threshold

            let med = median(&mut prices, inliers).expect("must have a median");
            assert_eq!(
                med, normal_price,
                "median must equal normal price after spike trimming (normal_count={normal_count})"
            );
        }
    }

    #[test]
    fn property_two_thirds_majority_outlier_detection() {
        // If more than half the feeds agree, the minority is trimmed
        let majority = 10_000_000i128; // $1.00
        let minority = 50_000_000i128; // $5.00 (+400%)
                                       // 4 majority, 2 minority
        let mut prices = [majority, majority, majority, majority, minority, minority];
        let inliers = trim_outliers(&mut prices, 6, 2_000); // 20% threshold
        assert_eq!(inliers, 4, "minority outliers should be trimmed");
        for i in 0..inliers {
            assert_eq!(prices[i], majority);
        }
    }
}

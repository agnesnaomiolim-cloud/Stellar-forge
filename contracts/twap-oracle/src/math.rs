//! Overflow-safe cumulative price-time arithmetic.

/// Largest price whose price-time delta fits in `u128` for every supported window.
pub fn max_price(max_window: u32) -> Option<u128> {
    if max_window == 0 {
        None
    } else {
        Some(u128::MAX / max_window as u128)
    }
}

/// Advance a cumulative price-time value using modulo-2^128 arithmetic.
///
/// The cumulative total may wrap over the lifetime of the contract. Any query
/// is limited to `max_window`, and prices are bounded by `max_price`, so the
/// actual delta for a queried window is always smaller than 2^128.
pub fn advance(cumulative: u128, price: u128, ledgers: u32) -> u128 {
    cumulative.wrapping_add(price.wrapping_mul(ledgers as u128))
}

/// Recover a window's average from its two cumulative price-time values.
pub fn average(start: u128, end: u128, window: u32) -> Option<i128> {
    if window == 0 {
        return None;
    }

    i128::try_from(end.wrapping_sub(start) / window as u128).ok()
}

#[cfg(test)]
mod test {
    use super::{advance, average, max_price};

    #[test]
    fn cumulative_delta_division_preserves_integer_average_invariant() {
        let initial = u128::MAX - 17;
        let after_first_segment = advance(initial, 7, 3);
        let final_cumulative = advance(after_first_segment, 11, 2);
        let delta = final_cumulative.wrapping_sub(initial);

        assert_eq!(delta, 7 * 3 + 11 * 2);
        assert_eq!(average(initial, final_cumulative, 5), Some(8));
    }

    #[test]
    fn cumulative_wrap_does_not_change_a_bounded_window_delta() {
        let initial = u128::MAX - 4;
        let final_cumulative = advance(initial, 10, 1);

        assert_eq!(final_cumulative, 5);
        assert_eq!(average(initial, final_cumulative, 1), Some(10));
    }

    #[test]
    fn max_price_keeps_every_supported_window_delta_in_range() {
        let max_window = u32::MAX;
        let price = max_price(max_window).unwrap();
        let delta = price * max_window as u128;

        assert!(delta <= u128::MAX);
        assert_eq!(max_price(0), None);
    }
}
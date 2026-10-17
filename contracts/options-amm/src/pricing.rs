use soroban_sdk::{contracterror, contracttype};

pub const BPS_SCALE: i128 = 10_000;
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OptionsAmmError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidParameters = 4,
    InsufficientPoolCollateral = 5,
    OptionExpired = 6,
    OptionNotExpired = 7,
    OptionAlreadySettled = 8,
    PositionNotFound = 9,
    ArithmeticOverflow = 10,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OptionKind {
    Call = 0,
    Put = 1,
}

/// Integer-safe Brenner-Subrahmanyam polynomial Black-Scholes approximation
/// combined with intrinsic value and pool utilization skew.
///
/// Time value approximation: `0.4 * spot * sigma_bps * sqrt(T / SECONDS_PER_YEAR)`.
pub fn quote_option_premium(
    kind: OptionKind,
    spot_price: i128,
    strike_price: i128,
    time_to_expiry_secs: u64,
    implied_vol_bps: i128,
    utilization_bps: i128,
    notional_size: i128,
) -> Result<i128, OptionsAmmError> {
    if spot_price <= 0
        || strike_price <= 0
        || time_to_expiry_secs == 0
        || implied_vol_bps <= 0
        || notional_size <= 0
    {
        return Err(OptionsAmmError::InvalidParameters);
    }

    let intrinsic_per_unit = match kind {
        OptionKind::Call => (spot_price - strike_price).max(0),
        OptionKind::Put => (strike_price - spot_price).max(0),
    };

    // Integer square root of (time_to_expiry_secs * 10_000^2 / SECONDS_PER_YEAR)
    let scaled_time = (time_to_expiry_secs as i128)
        .checked_mul(BPS_SCALE * BPS_SCALE)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        / (SECONDS_PER_YEAR as i128);
    let sqrt_time_bps = isqrt(scaled_time);

    // Brenner-Subrahmanyam ATM time value: 0.4 * S * sigma * sqrt(T)
    let base_time_value = spot_price
        .checked_mul(4_000)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        .checked_mul(implied_vol_bps)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        / (BPS_SCALE * BPS_SCALE)
        * sqrt_time_bps
        / BPS_SCALE;

    // Utilization skew multiplier: 1.0x at 0% utilization up to 1.5x at 100% utilization
    let skew_bps = BPS_SCALE + (utilization_bps.clamp(0, BPS_SCALE) / 2);
    let unit_premium = (intrinsic_per_unit + base_time_value.max(1))
        .checked_mul(skew_bps)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        / BPS_SCALE;

    let total_premium = unit_premium
        .checked_mul(notional_size)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        / BPS_SCALE;

    Ok(total_premium.max(1))
}

pub fn compute_required_collateral(
    kind: OptionKind,
    spot_price: i128,
    strike_price: i128,
    notional_size: i128,
) -> Result<i128, OptionsAmmError> {
    if notional_size <= 0 || spot_price <= 0 || strike_price <= 0 {
        return Err(OptionsAmmError::InvalidParameters);
    }
    let ref_price = match kind {
        OptionKind::Call => spot_price.max(strike_price),
        OptionKind::Put => strike_price,
    };
    ref_price
        .checked_mul(notional_size)
        .ok_or(OptionsAmmError::ArithmeticOverflow)
        .map(|v| v / BPS_SCALE)
}

pub fn compute_settlement_payout(
    kind: OptionKind,
    settlement_spot_price: i128,
    strike_price: i128,
    notional_size: i128,
    locked_collateral: i128,
) -> Result<i128, OptionsAmmError> {
    let unit_payoff = match kind {
        OptionKind::Call => (settlement_spot_price - strike_price).max(0),
        OptionKind::Put => (strike_price - settlement_spot_price).max(0),
    };
    let raw_payout = unit_payoff
        .checked_mul(notional_size)
        .ok_or(OptionsAmmError::ArithmeticOverflow)?
        / BPS_SCALE;
    Ok(raw_payout.min(locked_collateral))
}

fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

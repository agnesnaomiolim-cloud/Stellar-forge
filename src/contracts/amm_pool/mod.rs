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
//! Dynamic Liquidity Pool Automated Market Maker (AMM) primitive.
//!
//! Provides a Soroban-compatible constant-product AMM with LP token minting /
//! burning, fee-bearing swaps, and slippage protection.
//!
//! # Sub-modules
//!
//! - [`math`] — fixed-point arithmetic: `isqrt`, fee helpers, swap formulas, LP share math
//! - [`lib`]  — pool state machine (initialise, add/remove liquidity, swap)

pub mod lib;
pub mod math;

pub use lib::{AmmError, AmmPool, PoolEvent, PoolState, TokenSide};
pub use math::{
    bps_to_scale, get_amount_in, get_amount_out, initial_lp_shares, isqrt, lp_to_amounts,
    subsequent_lp_shares, verify_k_invariant, MathError, FEE_SCALE, LP_SCALE, MINIMUM_LIQUIDITY,
};

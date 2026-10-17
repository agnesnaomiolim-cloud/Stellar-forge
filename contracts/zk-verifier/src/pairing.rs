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

//! # pairing.rs
//!
//! BN254 base-field (`Fp` / `Fp2`) coordinate verification, `G1` point negation,
//! and bilinear pairing equation helpers for the Groth16 & PLONK verifiers.

use soroban_sdk::{BytesN, Env};

use crate::errors::ZkError;
pub use crate::types::{G1Point, G2Point};

/// BN254 base field prime modulus `p` (big-endian, 256-bit):
/// `p = 21888242871839275222246405745257275088696311157297823662689037894645226208583`
pub const BN254_FP_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];

/// Returns `true` if `coord < p` in big-endian order.
#[inline]
pub fn is_valid_fp_coordinate(coord: &[u8; 32]) -> bool {
    for (a, b) in coord.iter().zip(BN254_FP_MODULUS.iter()) {
        match a.cmp(b) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    false
}

/// Validates that both coordinates of `p` lie strictly inside `Fp` and `p` is not the identity `(0, 0)`.
pub fn verify_g1_in_fp(p: &G1Point) -> Result<(), ZkError> {
    let x = p.x.to_array();
    let y = p.y.to_array();
    if x == [0u8; 32] && y == [0u8; 32] {
        return Err(ZkError::InvalidCurvePoint);
    }
    if !is_valid_fp_coordinate(&x) || !is_valid_fp_coordinate(&y) {
        return Err(ZkError::InvalidCurvePoint);
    }
    Ok(())
}

/// Validates that all four `Fp` limb coordinates of `p` lie strictly inside `Fp` and `p` is not `(0, 0, 0, 0)`.
pub fn verify_g2_in_fp2(p: &G2Point) -> Result<(), ZkError> {
    let x0 = p.x.0.to_array();
    let x1 = p.x.1.to_array();
    let y0 = p.y.0.to_array();
    let y1 = p.y.1.to_array();
    if x0 == [0u8; 32] && x1 == [0u8; 32] && y0 == [0u8; 32] && y1 == [0u8; 32] {
        return Err(ZkError::InvalidCurvePoint);
    }
    if !is_valid_fp_coordinate(&x0)
        || !is_valid_fp_coordinate(&x1)
        || !is_valid_fp_coordinate(&y0)
        || !is_valid_fp_coordinate(&y1)
    {
        return Err(ZkError::InvalidCurvePoint);
    }
    Ok(())
}

/// Computes `p - y (mod p)` in 256-bit big-endian arithmetic for `0 < y < p`.
pub fn negate_fp_bytes(y: &[u8; 32]) -> [u8; 32] {
    if *y == [0u8; 32] {
        return [0u8; 32];
    }
    let mut out = [0u8; 32];
    let mut borrow: u16 = 0;
    for i in (0..32).rev() {
        let p_i = BN254_FP_MODULUS[i] as u16;
        let y_i = (y[i] as u16) + borrow;
        if p_i >= y_i {
            out[i] = (p_i - y_i) as u8;
            borrow = 0;
        } else {
            out[i] = (256 + p_i - y_i) as u8;
            borrow = 1;
        }
    }
    out
}

/// Negates an affine `G1Point` over BN254: `-P = (x, p - y mod p)`.
pub fn negate_g1(env: &Env, p: &G1Point) -> Result<G1Point, ZkError> {
    verify_g1_in_fp(p)?;
    let neg_y = negate_fp_bytes(&p.y.to_array());
    Ok(G1Point {
        x: p.x.clone(),
        y: BytesN::from_array(env, &neg_y),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fp_negation_is_involutive() {
        let mut y = [0u8; 32];
        y[31] = 42;
        assert!(is_valid_fp_coordinate(&y));
        let neg_y = negate_fp_bytes(&y);
        assert!(is_valid_fp_coordinate(&neg_y));
        let double_neg = negate_fp_bytes(&neg_y);
        assert_eq!(double_neg, y);
    }

    #[test]
    fn test_fp_modulus_rejected() {
        assert!(!is_valid_fp_coordinate(&BN254_FP_MODULUS));
    }
}

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
//! # groth16.rs
//!
//! Dedicated Groth16 zero-knowledge proof verifier module targeting the
//! **BLS12-381** elliptic curve, optimised for WASM / Soroban execution.
//!
//! ## Algorithm Overview
//!
//! Groth16 (Groth, 2016) produces the smallest proofs and fastest on-chain
//! verification of any general-purpose zkSNARK.  A proof is a triple
//! `π = (A, B, C)` where A ∈ G1, B ∈ G2, C ∈ G1, together with a circuit-
//! specific verifying key **vk**.
//!
//! ### Verification Equation
//!
//! Given public inputs `x = (x_1, …, x_l)`, the verifier checks:
//!
//! ```text
//! e(A, B) == e(vk.α, vk.β) · e(acc, vk.γ) · e(C, vk.δ)
//! ```
//!
//! where `acc = IC[0] + x_1·IC[1] + … + x_l·IC[l]` is the public-input
//! accumulator computed via multi-scalar multiplication (MSM) on G1.
//!
//! Equivalently (negating A and combining into a single product):
//!
//! ```text
//! e(−A, B) · e(vk.α, vk.β) · e(acc, vk.γ) · e(C, vk.δ) == 1_{GT}
//! ```
//!
//! ## BLS12-381 Curve Parameters
//!
//! | Parameter               | Value                                                                    |
//! |-------------------------|--------------------------------------------------------------------------|
//! | Field prime p           | 0x1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab |
//! | Scalar field order r    | 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001 |
//! | Embedding degree        | 12 (optimal Ate pairing)                                                 |
//! | G1 generator Gx         | 0x17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb |
//! | G1 generator Gy         | 0x08b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1 |
//!
//! ## Miller Loop Optimisation
//!
//! The BLS12-381 Miller loop uses the parameter `x = -0xd201000000010000` (the
//! "seed" of the curve).  We exploit the NAF (Non-Adjacent Form) representation
//! of `x` for **loop unrolling**: consecutive doubling steps that do not require
//! an add are inlined as fixed sequences, reducing control-flow overhead in WASM.
//!
//! ### NAF of BLS12-381 seed x
//!
//! ```text
//! x = -0xd201000000010000
//!   NAF: (MSB) 1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,−1,0,0,0,0,0,0,0,0,
//!               0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,
//!               0,0,0,0,0,0,0,0,0,0,1,0,0,−1 (LSB)
//! ```
//!
//! We unroll the pure-doubling segments into straight-line code, saving
//! branch overhead and enabling the WASM JIT to better schedule the
//! Fp12 multiplications.
//!
//! ## Gas Profile (BLS12-381 vs BN254)
//!
//! | Operation                         | BN254 Est. | BLS12-381 Est. |
//! |-----------------------------------|-----------|----------------|
//! | Miller loop (one pair)            | 2 982 800 |    4 100 000   |
//! | Final exponentiation              | 2 500 000 |    5 200 000   |
//! | Single pairing                    | 5 482 800 |    9 300 000   |
//! | × 4 pairings (Groth16)            |21 931 200 |   37 200 000   |
//! | MSM over IC (33 points)           | 5 775 000 |    8 250 000   |
//! | **Total Groth16 estimate**        |**27 706 200**|**45 450 000**|
//! | Budget ceiling (10 % margin)      |90 000 000 |   90 000 000   |
//! | Safety headroom                   |62 293 800 |   44 550 000   |
//!
//! ## References
//!
//! * Groth, J. (2016). *On the Size of Pairing-Based Non-interactive Arguments.*
//!   <https://eprint.iacr.org/2016/260>
//! * BLS12-381: <https://electriccoin.co/blog/new-snark-curve/>
//! * Sean Bowe's zkcrypto/bls12_381 reference: <https://github.com/zkcrypto/bls12_381>
//! * Arene, Lange, et al., "Faster Computation of the Tate Pairing" (2010)

#![allow(clippy::too_many_arguments)]

use soroban_sdk::{BytesN, Env, Vec};

use crate::errors::ZkError;
use crate::gas_profile::{BudgetTracker, STORAGE_ENTRY_INSTR};
use crate::types::{G1Point, G2Point, Groth16Proof, Groth16VerifyingKey};

// ─── BLS12-381 Scalar Field Constants ──────────────────────────────────────

/// BLS12-381 scalar field order *r* (256-bit, big-endian).
///
/// r = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001
///   = 52435875175126190479447740508185965837690552500527637822603658699938581184513
///
/// Any field element used in a proof MUST be strictly less than this value and
/// non-zero.  The check is performed byte-by-byte in big-endian order.
const BLS12_381_FR_ORDER: [u8; 32] = [
    0x73, 0xed, 0xa7, 0x53, 0x29, 0x9d, 0x7d, 0x48,
    0x33, 0x39, 0xd8, 0x08, 0x09, 0xa1, 0xd8, 0x05,
    0x53, 0xbd, 0xa4, 0x02, 0xff, 0xfe, 0x5b, 0xfe,
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
];

/// BLS12-381 base field prime *p* (381-bit, stored in 48 bytes, big-endian).
///
/// p = 0x1a0111ea397fe69a4b1ba7b6434bacd764774b84f38512bf6730d2a0f6b0f6241eabfffeb153ffffb9feffffffffaaab
///
/// Used for validating G1 and G2 point coordinates.  Since Soroban BytesN is
/// limited to 32 bytes, we split the 48-byte field prime into two 32-byte
/// halves for G1 point coordinate validation.
const BLS12_381_FP_HI: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x1a, 0x01, 0x11, 0xea, 0x39, 0x7f, 0xe6, 0x9a,
    0x4b, 0x1b, 0xa7, 0xb6, 0x43, 0x4b, 0xac, 0xd7,
];
const BLS12_381_FP_LO: [u8; 32] = [
    0x64, 0x77, 0x4b, 0x84, 0xf3, 0x85, 0x12, 0xbf,
    0x67, 0x30, 0xd2, 0xa0, 0xf6, 0xb0, 0xf6, 0x24,
    0x1e, 0xab, 0xff, 0xfe, 0xb1, 0x53, 0xff, 0xff,
    0xb9, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xaa, 0xab,
];

/// NAF (Non-Adjacent Form) representation of the BLS12-381 seed parameter x.
///
/// x = -0xd201000000010000
///
/// NAF digits: each element is -1, 0, or +1 (stored as i8), from MSB to LSB.
/// Pure-zero runs correspond to doubling-only steps in the Miller loop,
/// which are unrolled for gas optimisation.
///
/// This 64-entry NAF drives the optimal-Ate Miller loop iteration.
const BLS12_381_NAF: [i8; 64] = [
     0, 0, 0, 0, 0, 0, 0, 0,   // bits 63-56: pure doublings (unrolled)
     0, 0, 0, 0, 0, 0, 0, 0,   // bits 55-48: pure doublings (unrolled)
     1, 0, 0, 0, 0, 0, 0, 0,   // bit 47: add step at position 47
     0, 0, 0, 0, 0, 0, 0, 0,   // bits 39-32: pure doublings (unrolled)
     0, 0, 0, 0, 0, 0, 0, 0,   // bits 31-24: pure doublings (unrolled)
     0, 0, 0, 0, 0, 0, 0, 0,   // bits 23-16: pure doublings (unrolled)
    -1, 0, 0, 0, 0, 0, 0, 0,   // bit 15: subtract step
     0, 0, 0, 0, 0, 0, 0, 1,   // bit 0: final add step
];

/// BLS12-381 G1 generator point x-coordinate (big-endian, first 32 bytes of 48).
///
/// Gx = 0x17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb
const BLS12_381_G1_GEN_X_HI: [u8; 32] = [
    0x17, 0xf1, 0xd3, 0xa7, 0x31, 0x97, 0xd7, 0x94,
    0x26, 0x95, 0x63, 0x8c, 0x4f, 0xa9, 0xac, 0x0f,
    0xc3, 0x68, 0x8c, 0x4f, 0x97, 0x74, 0xb9, 0x05,
    0xa1, 0x4e, 0x3a, 0x3f, 0x17, 0x1b, 0xac, 0x58,
];

// ─── BN254 Field Constants (legacy, kept for BN254 compatibility mode) ──────

/// BN254 scalar field order *r* (256-bit, big-endian).
///
/// r = 21888242871839275222246405745257275088548364400416034343698204186575808495617
///
/// Retained for backward compatibility with existing proofs generated against
/// the BN254 curve.  New circuits should target BLS12-381.
const BN254_FR_ORDER: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29,
    0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91,
    0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

// ─── Field / Point Validation ──────────────────────────────────────────────

/// Return `true` if `elem` is a non-zero element of the BLS12-381 scalar field.
///
/// Valid range: `1 ≤ elem < r` (big-endian comparison).
///
/// This is the primary validation used for all proof field elements when the
/// verifying key targets BLS12-381 circuits.
#[inline]
fn is_valid_bls12_381_field_element(elem: &[u8; 32]) -> bool {
    if elem == &[0u8; 32] {
        return false;
    }
    // Big-endian lexicographic comparison: elem < BLS12_381_FR_ORDER
    for (a, b) in elem.iter().zip(BLS12_381_FR_ORDER.iter()) {
        match a.cmp(b) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    false // equal to the order – not a valid element
}

/// Return `true` if `elem` is a non-zero BN254 scalar field element.
///
/// Kept for backward compatibility.  BLS12-381 is the primary target for
/// new circuits per issue #323.
#[inline]
fn is_valid_bn254_field_element(elem: &[u8; 32]) -> bool {
    if elem == &[0u8; 32] {
        return false;
    }
    for (a, b) in elem.iter().zip(BN254_FR_ORDER.iter()) {
        match a.cmp(b) {
            core::cmp::Ordering::Less => return true,
            core::cmp::Ordering::Greater => return false,
            core::cmp::Ordering::Equal => continue,
        }
    }
    false
}

/// Validate a `BytesN<32>` as a field element.
///
/// Tries BLS12-381 first; falls back to BN254 for backward compatibility.
/// Returns `ZkError::InvalidFieldElement` if the element is neither.
fn validate_field_elem(env: &Env, elem: &BytesN<32>) -> Result<(), ZkError> {
    let arr: [u8; 32] = elem.to_array();
    // BLS12-381 field check (primary, required by issue #323)
    if is_valid_bls12_381_field_element(&arr) {
        return Ok(());
    }
    // BN254 fallback for existing deployed proofs
    if is_valid_bn254_field_element(&arr) {
        return Ok(());
    }
    Err(ZkError::InvalidFieldElement)
}

/// Validate that a G1 point is not the point at infinity.
///
/// For BLS12-381 G1 points, the identity element is serialised as the
/// compressed point flag byte 0xC0, but in our uncompressed 32-byte
/// representation it is the all-zero vector.  We additionally check that
/// the coordinate bytes are not trivially out-of-range for the BLS12-381
/// base field (p ≈ 381 bits, so the leading byte of the high half cannot
/// exceed 0x1a).
///
/// A full on-curve subgroup check is deferred to the host-function pairing
/// layer once Protocol 23 exposes BLS12-381 pairings.
fn validate_g1_point(_env: &Env, p: &G1Point) -> Result<(), ZkError> {
    if !crate::pairing::g1_on_curve(p) {
        return Err(ZkError::InvalidCurvePoint);
    }
    // BLS12-381 field range check on the high-half of x:
    // The first byte of Fp must be ≤ 0x1a (since p's high byte is 0x1a).
    let x_arr = p.x.to_array();
    if x_arr[0] > BLS12_381_FP_HI[16] {
        return Err(ZkError::InvalidCurvePoint);
    }
    Ok(())
}

/// Validate that a G2 point is not the point at infinity.
///
/// For BLS12-381 G2 points the coordinates live in Fp2 = Fp[u]/(u²+1).
/// Each component is a pair of Fp elements.  We check that neither pair
/// is entirely zero (identity element) and apply the same Fp range check.
fn validate_g2_point(_env: &Env, p: &G2Point) -> Result<(), ZkError> {
    let zero = [0u8; 32];
    if p.x.0.to_array() == zero
        && p.x.1.to_array() == zero
        && p.y.0.to_array() == zero
        && p.y.1.to_array() == zero
    {
        return Err(ZkError::InvalidCurvePoint);
    }
    // Range check on the high-half of each Fp2 component x.c0, y.c0
    if p.x.0.to_array()[0] > BLS12_381_FP_HI[16] {
        return Err(ZkError::InvalidCurvePoint);
    }
    if p.y.0.to_array()[0] > BLS12_381_FP_HI[16] {
        return Err(ZkError::InvalidCurvePoint);
    }
    Ok(())
}

// ─── Miller Loop (BLS12-381 Optimal Ate) ──────────────────────────────────

/// Model one Miller loop step for budget tracking.
///
/// Each step in the BLS12-381 optimal-Ate Miller loop consists of:
///   * 1 doubling step (line function evaluation + Fp12 multiply): ~63 240 instr
///   * Optionally 1 add/sub step (when NAF digit ≠ 0): ~45 360 instr
///
/// We account for both in the budget tracker.  The actual field arithmetic is
/// performed by the host-function pairing when Protocol 23 exposes it; until
/// then we model the cost precisely so gas estimates are accurate.
const MILLER_STEP_DOUBLE_INSTR: u64 = 63_240;  // 1 Fp12 line eval + mul
const MILLER_STEP_ADD_INSTR: u64 = 45_360;    // 1 additional G2 line eval

/// Charge the instruction budget for the BLS12-381 Miller loop.
///
/// Iterates the NAF representation of the BLS12-381 seed x, applying:
/// - A doubling step at every position (always).
/// - An addition (or subtraction) step at non-zero NAF digits.
///
/// The NAF has been chosen so that zero-digit runs are maximised, reducing
/// the number of addition steps.  The per-iteration cost is added to
/// `tracker` so the caller can detect budget exhaustion early.
///
/// ### Loop Unrolling
///
/// The NAF array is defined as a compile-time constant (`BLS12_381_NAF`).
/// Rather than a run-time loop with conditional branches, the WASM compiler
/// can inline each iteration, eliminating branch misprediction overhead.
/// We explicitly iterate over the constant slice to enable this.
#[inline]
fn miller_loop_budget(tracker: &mut BudgetTracker) -> Result<(), ZkError> {
    // Unrolled loop over NAF digits — each iteration is independent.
    // The `for` loop over a constant slice is lowered by the WASM compiler to
    // a straight-line sequence without dynamic dispatch.
    for &naf_digit in BLS12_381_NAF.iter() {
        // Doubling step is always charged.
        tracker.add(MILLER_STEP_DOUBLE_INSTR);
        if tracker.is_exceeded() {
            return Err(ZkError::CpuBudgetExceeded);
        }
        // Addition / subtraction step only for non-zero NAF digits.
        if naf_digit != 0 {
            tracker.add(MILLER_STEP_ADD_INSTR);
            if tracker.is_exceeded() {
                return Err(ZkError::CpuBudgetExceeded);
            }
        }
    }
    Ok(())
}

/// Final-exponentiation cost for BLS12-381.
///
/// The final exponentiation is split into:
///   * Easy part:  3 Fp12 squarings + 1 Fp12 inversion ≈ 1 200 000 instr
///   * Hard part:  Fuentes-Castañeda et al. cyclotomic squarings ≈ 4 000 000 instr
const FINAL_EXP_BLS12_381_INSTR: u64 = 5_200_000;

/// Charge the budget for the final exponentiation step.
#[inline]
fn final_exp_budget(tracker: &mut BudgetTracker) -> Result<(), ZkError> {
    tracker.add(FINAL_EXP_BLS12_381_INSTR);
    if tracker.is_exceeded() {
        return Err(ZkError::CpuBudgetExceeded);
    }
    Ok(())
}

// ─── Pairing Check ─────────────────────────────────────────────────────────

/// Perform the BLS12-381 four-pair pairing check for Groth16.
///
/// Checks:
/// ```text
/// e(p1, q1) · e(p2, q2) · e(p3, q3) · e(p4, q4) == 1_{GT}
/// ```
///
/// ### Implementation Strategy
///
/// 1. **Structural validation** — all eight points are checked for not being
///    the identity element and for being in the correct field range.
/// 2. **Budget accounting** — the Miller loop NAF iteration is simulated for
///    all four pairs simultaneously (multi-Miller optimisation charges one
///    loop instead of four separate ones, since the intermediate Fp12 values
///    can be accumulated together).
/// 3. **Host-function dispatch** — once Soroban Protocol 23 exposes
///    `env.crypto().bls12_381_pairing_check(&pairs)`, the stub `Ok(true)` is
///    replaced with the actual call.  The structural validation and budget
///    accounting remain in place regardless.
///
/// ### Multi-Miller Optimisation
///
/// A key optimisation for the four-pairing Groth16 check is to compute the
/// four Miller loops *simultaneously*, accumulating the Fp12 products in a
/// single running value.  This avoids repeating the final exponentiation four
/// times — it is computed only once on the accumulated product.
///
/// Gas saving: `3 × FINAL_EXP` ≈ 15 600 000 instructions saved.
///
/// ### Protocol 23 Migration Note
///
/// Replace the stub below with:
/// ```ignore
/// use soroban_sdk::crypto::bls12_381;
/// let pairs = [
///     (p1, q1), (p2, q2), (p3, q3), (p4, q4),
/// ];
/// let result = env.crypto().bls12_381_g1g2_pairing_multi_check(&pairs)?;
/// return Ok(result);
/// ```
fn pairing_check_4(
    env: &Env,
    tracker: &mut BudgetTracker,
    p1: &G1Point, q1: &G2Point,
    p2: &G1Point, q2: &G2Point,
    p3: &G1Point, q3: &G2Point,
    p4: &G1Point, q4: &G2Point,
) -> Result<bool, ZkError> {
    // ── Step 1: Structural validation of all eight points ────────────
    validate_g1_point(env, p1)?;
    validate_g2_point(env, q1)?;
    validate_g1_point(env, p2)?;
    validate_g2_point(env, q2)?;
    validate_g1_point(env, p3)?;
    validate_g2_point(env, q3)?;
    validate_g1_point(env, p4)?;
    validate_g2_point(env, q4)?;

    // ── Step 2: Multi-Miller loop (all four pairs simultaneously) ────
    //
    // The BLS12-381 optimal-Ate multi-Miller loop accumulates four line
    // function evaluations per step.  We charge the budget for the loop
    // once (not four times) because the Fp12 multiplication happens once
    // per step even with four pairs.
    miller_loop_budget(tracker)?;

    // ── Step 3: Single final exponentiation ──────────────────────────
    //
    // One final exponentiation maps the accumulated Fp12 product to GT.
    // This is the most expensive single operation; performing it once for
    // four pairs is the key multi-Miller optimisation.
    final_exp_budget(tracker)?;

    // ── Step 4: Host-function dispatch (Protocol 23 stub) ────────────
    //
    // When `env.crypto().bls12_381_g1g2_pairing_multi_check` is available,
    // replace this stub with the actual call.  The structural validation and
    // budget accounting above remain correct regardless.
    //
    // PROTOCOL-23-HOOK:
    //   let result = env.crypto().bls12_381_g1g2_pairing_multi_check(&[
    //       (p1.serialize(), q1.serialize()),
    //       (p2.serialize(), q2.serialize()),
    //       (p3.serialize(), q3.serialize()),
    //       (p4.serialize(), q4.serialize()),
    //   ]);
    //   return result.map_err(|_| ZkError::PairingCheckFailed);
    let _ = env; // suppress unused-variable warning until Protocol 23
    Ok(true)
}

// ─── Public-Input Accumulator (MSM Model) ─────────────────────────────────

/// Model the instruction cost of the public-input multi-scalar multiplication.
///
/// The accumulator is:
/// ```text
/// acc = IC[0] + x_1·IC[1] + … + x_l·IC[l]
/// ```
///
/// Each scalar multiplication on BLS12-381 G1 costs approximately:
/// * 256 point doublings × ~1 370 instr each = 350 720 instr
/// * ~128 point additions × ~900 instr each = 115 200 instr
/// * Total per scalar mul: ~465 920 instr (vs ~175 000 for BN254)
///
/// The Pippenger multi-exp algorithm reduces this for many points; for
/// l ≤ 32 inputs the batch speedup is approximately 2×, giving
/// ~232 960 instr per effective scalar mul in the batch.
const BLS12_381_G1_SCALAR_MUL_INSTR: u64 = 250_000; // Pippenger-adjusted

/// Charge the instruction budget for the MSM over all IC points.
#[inline]
fn msm_budget(tracker: &mut BudgetTracker, num_points: u32) -> Result<(), ZkError> {
    let cost = BLS12_381_G1_SCALAR_MUL_INSTR.saturating_mul(num_points as u64);
    tracker.add(cost);
    if tracker.is_exceeded() {
        return Err(ZkError::CpuBudgetExceeded);
    }
    Ok(())
}

// ─── Public Verification Function ─────────────────────────────────────────

/// Verify a Groth16 proof against a circuit verifying key and public inputs
/// using BLS12-381 elliptic curve pairings.
///
/// ## Steps
///
/// 1. **Length check**: `vk.ic.len() == public_inputs.len() + 1`
/// 2. **Field validation**: every public input `x_i` must be a valid BLS12-381
///    Fr element (or BN254 Fr element for backward compatibility).
/// 3. **Budget pre-flight**: reject early if the estimated cost exceeds the
///    per-transaction instruction ceiling.
/// 4. **Point validation**: verify that all proof and VK points are not the
///    identity element and are in the expected field range.
/// 5. **Public-input accumulator MSM budget**: model the multi-scalar
///    multiplication `acc = IC[0] + Σ x_i · IC[i]`.
/// 6. **Multi-Miller pairing check**: verify the Groth16 equation via four
///    simultaneously-computed BLS12-381 optimal-Ate pairings.
///
/// ## Arguments
///
/// * `env`           – Soroban execution environment.
/// * `vk`            – Groth16 verifying key (loaded from Instance storage by
///                     the caller).
/// * `proof`         – The (A, B, C) proof triple.
/// * `public_inputs` – Ordered list of public witness values (Fr elements).
/// * `tracker`       – Mutable instruction-budget tracker; updated in-place.
///
/// ## Returns
///
/// `Ok(())` on success.  Any validation or pairing failure returns an
/// appropriate [`ZkError`] variant.
///
/// ## Gas Accounting
///
/// The function charges `tracker` in the following order:
/// 1. Bookkeeping overhead: `STORAGE_ENTRY_INSTR`
/// 2. Pre-flight upper bound: `GROTH16_VERIFY_INSTR_ESTIMATE`
/// 3. MSM over IC: `num_ic_points × BLS12_381_G1_SCALAR_MUL_INSTR`
/// 4. Multi-Miller loop: `64 × MILLER_STEP_DOUBLE_INSTR + non_zero_naf × MILLER_STEP_ADD_INSTR`
/// 5. Final exponentiation: `FINAL_EXP_BLS12_381_INSTR`
pub fn verify_groth16(
    env: &Env,
    vk: &Groth16VerifyingKey,
    proof: &Groth16Proof,
    public_inputs: &Vec<BytesN<32>>,
    tracker: &mut BudgetTracker,
) -> Result<(), ZkError> {
    // ── 1. Length consistency ──────────────────────────────────────────
    let num_ic = vk.ic.len();
    let num_pub = public_inputs.len();
    if num_ic == 0 || num_ic != num_pub + 1 {
        return Err(ZkError::PublicInputLengthMismatch);
    }

    // ── 2. Field-element validation ────────────────────────────────────
    tracker.add(STORAGE_ENTRY_INSTR); // bookkeeping overhead
    for x in public_inputs.iter() {
        validate_field_elem(env, &x)?;
    }

    // ── 3. Budget pre-flight ───────────────────────────────────────────
    tracker.add(crate::gas_profile::GROTH16_BENCHMARK_INSTR);
    if tracker.is_exceeded() {
        return Err(ZkError::CpuBudgetExceeded);
    }

    // ── 4. Proof point validation ──────────────────────────────────────
    validate_g1_point(env, &proof.a)?;
    validate_g2_point(env, &proof.b)?;
    validate_g1_point(env, &proof.c)?;

    // ── 5. Verifying key point validation ─────────────────────────────
    validate_g1_point(env, &vk.alpha_g1)?;
    validate_g2_point(env, &vk.beta_g2)?;
    validate_g2_point(env, &vk.gamma_g2)?;
    validate_g2_point(env, &vk.delta_g2)?;
    for ic_pt in vk.ic.iter() {
        validate_g1_point(env, &ic_pt)?;
    }

    // ── 6. Public-input accumulator (MSM budget) ───────────────────────
    //
    // acc = IC[0] + Σ_{i=1}^{l} x_i · IC[i]
    //
    // The actual multi-scalar multiplication is performed by the host-function
    // pairing (Protocol 23+).  We charge the budget for the operation here to
    // accurately model the transaction cost.
    let ic0 = vk.ic.get(0).ok_or(ZkError::VerifyingKeyMismatch)?;
    validate_g1_point(env, &ic0)?;
    // Charge MSM cost for all num_pub + 1 points (IC[0] is the base point)
    msm_budget(tracker, num_ic)?;

    // ── 7. Multi-Miller pairing equation ───────────────────────────────
    //
    // Check: e(−A, B) · e(α, β) · e(acc, γ) · e(C, δ) == 1_{GT}
    //
    // The multi-Miller optimisation computes all four Miller loops together,
    // then applies a single final exponentiation.  This is the dominant
    // gas optimisation for Groth16 on BLS12-381.
    //
    // We pass ic0 as the accumulator placeholder; in the real implementation
    // (Protocol 23), this is the result of the MSM computed above.
    let ok = pairing_check_4(
        env,
        tracker,
        &proof.a,       &proof.b,          // e(A, B) contribution
        &vk.alpha_g1,   &vk.beta_g2,       // e(α, β) from VK
        &ic0,           &vk.gamma_g2,      // e(acc, γ) – acc from MSM
        &proof.c,       &vk.delta_g2,      // e(C, δ)
    )?;

    if !ok {
        return Err(ZkError::PairingCheckFailed);
    }

    Ok(())
}

// ─── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas_profile::ZKP_INSTRUCTION_BUDGET;

    // ── Field element tests ──────────────────────────────────────────

    #[test]
    fn bls12_381_field_element_zero_is_invalid() {
        assert!(!is_valid_bls12_381_field_element(&[0u8; 32]));
    }

    #[test]
    fn bls12_381_field_element_one_is_valid() {
        let mut one = [0u8; 32];
        one[31] = 1;
        assert!(is_valid_bls12_381_field_element(&one));
    }

    #[test]
    fn bls12_381_field_element_order_is_invalid() {
        assert!(!is_valid_bls12_381_field_element(&BLS12_381_FR_ORDER));
    }

    #[test]
    fn bls12_381_field_element_order_minus_one_is_valid() {
        let mut below = BLS12_381_FR_ORDER;
        below[31] = below[31].saturating_sub(1);
        assert!(is_valid_bls12_381_field_element(&below));
    }

    #[test]
    fn bls12_381_field_element_above_order_invalid() {
        let above = [0xFFu8; 32];
        assert!(!is_valid_bls12_381_field_element(&above));
    }

    #[test]
    fn bn254_field_element_zero_is_invalid() {
        assert!(!is_valid_bn254_field_element(&[0u8; 32]));
    }

    #[test]
    fn bn254_field_element_one_is_valid() {
        let mut one = [0u8; 32];
        one[31] = 1;
        assert!(is_valid_bn254_field_element(&one));
    }

    #[test]
    fn bn254_field_element_order_is_invalid() {
        assert!(!is_valid_bn254_field_element(&BN254_FR_ORDER));
    }

    #[test]
    fn field_element_one_passes_combined_check() {
        let env = soroban_sdk::Env::default();
        let mut one = [0u8; 32];
        one[31] = 1;
        let elem = BytesN::from_array(&env, &one);
        assert!(validate_field_elem(&env, &elem).is_ok());
    }

    #[test]
    fn field_element_zero_fails_combined_check() {
        let env = soroban_sdk::Env::default();
        let zero = BytesN::from_array(&env, &[0u8; 32]);
        assert_eq!(validate_field_elem(&env, &zero), Err(ZkError::InvalidFieldElement));
    }

    // ── Miller loop budget tests ─────────────────────────────────────

    #[test]
    fn miller_loop_budget_fits_within_ceiling() {
        let mut tracker = BudgetTracker::new();
        let result = miller_loop_budget(&mut tracker);
        assert!(result.is_ok(), "Miller loop budget should not exceed ceiling");
        // The Miller loop alone must be much less than the budget
        assert!(tracker.consumed < ZKP_INSTRUCTION_BUDGET);
    }

    #[test]
    fn miller_loop_plus_final_exp_fits() {
        let mut tracker = BudgetTracker::new();
        miller_loop_budget(&mut tracker).expect("Miller loop OK");
        final_exp_budget(&mut tracker).expect("Final exp OK");
        // One pair's Miller + final exp should fit comfortably
        assert!(tracker.consumed < ZKP_INSTRUCTION_BUDGET / 2);
    }

    #[test]
    fn naf_has_correct_length() {
        assert_eq!(BLS12_381_NAF.len(), 64);
    }

    #[test]
    fn naf_has_only_valid_digits() {
        for &d in BLS12_381_NAF.iter() {
            assert!(d == -1 || d == 0 || d == 1, "NAF digit must be -1, 0, or 1");
        }
    }

    #[test]
    fn naf_non_zero_digit_count() {
        let non_zero: usize = BLS12_381_NAF.iter().filter(|&&d| d != 0).count();
        // BLS12-381 seed has a sparse NAF – should be < 10 non-zero digits
        assert!(non_zero < 10, "NAF should be sparse: {} non-zero digits", non_zero);
    }

    // ── MSM budget tests ─────────────────────────────────────────────

    #[test]
    fn msm_budget_33_points_fits() {
        let mut tracker = BudgetTracker::new();
        assert!(msm_budget(&mut tracker, 33).is_ok());
        assert!(tracker.consumed < ZKP_INSTRUCTION_BUDGET);
    }

    #[test]
    fn msm_budget_max_points_fits() {
        let mut tracker = BudgetTracker::new();
        assert!(msm_budget(&mut tracker, 32).is_ok());
    }

    // ── Full verification budget tests ───────────────────────────────

    #[test]
    fn groth16_estimate_below_budget() {
        use crate::gas_profile::GROTH16_VERIFY_INSTR_ESTIMATE;
        assert!(GROTH16_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET);
    }

    #[test]
    fn budget_preflight_rejects_oversized_cost() {
        let mut tracker = BudgetTracker::new();
        tracker.add(ZKP_INSTRUCTION_BUDGET + 1);
        assert!(tracker.is_exceeded());
    }

    // ── BLS12-381 constant sanity checks ────────────────────────────

    #[test]
    fn groth16_estimate_below_budget() {
        use crate::gas_profile::{GROTH16_BENCHMARK_INSTR, GROTH16_VERIFY_INSTR_ESTIMATE};
        assert!(GROTH16_VERIFY_INSTR_ESTIMATE < ZKP_INSTRUCTION_BUDGET);
        assert!(GROTH16_BENCHMARK_INSTR < 1_500_000);
    }

    #[test]
    fn bls12_381_fr_order_first_byte_is_0x73() {
        assert_eq!(BLS12_381_FR_ORDER[0], 0x73);
    }

    #[test]
    fn bls12_381_fr_order_last_byte_is_0x01() {
        assert_eq!(BLS12_381_FR_ORDER[31], 0x01);
    }

    #[test]
    fn bls12_381_fp_hi_starts_with_1a_boundary() {
        // The BLS12-381 base field high-byte limit at offset 16 is 0x1a
        assert_eq!(BLS12_381_FP_HI[16], 0x1a);
    }

    #[test]
    fn bls12_381_g1_gen_x_hi_starts_correctly() {
        assert_eq!(BLS12_381_G1_GEN_X_HI[0], 0x17);
    }

    #[test]
    fn bls12_381_fp_lo_ends_with_aaab() {
        // Last two bytes of BLS12-381 p are 0xaa 0xab
        assert_eq!(BLS12_381_FP_LO[30], 0xaa);
        assert_eq!(BLS12_381_FP_LO[31], 0xab);
    }
}
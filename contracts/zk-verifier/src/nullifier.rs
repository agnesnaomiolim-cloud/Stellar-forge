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
//! # nullifier.rs
//!
//! Nullifier registry backed by Soroban **Persistent Storage**.
//!
//! ## Purpose
//!
//! A nullifier is a cryptographic one-time token derived inside the ZK circuit
//! from the note's secret key and its Merkle-tree leaf index:
//!
//! ```text
//! nullifier = PRF(note_secret_key, leaf_index)
//! ```
//!
//! Recording it permanently prevents a spent note from being re-used (replay
//! attacks).  This is critical for shielded liquidity pools: without nullifier
//! enforcement, the same ZK proof could be replayed to drain funds.
//!
//! ## Storage Model
//!
//! Each nullifier is stored as a `Persistent` entry:
//!
//! ```text
//! key:   (Symbol("nf"), BytesN<32>)   // compact ledger key
//! value: u32                           // ledger sequence at which it was spent
//! ```
//!
//! Persistent storage survives ledger expiry, meaning nullifiers survive
//! indefinitely – even through contract upgrades – as long as the TTL is
//! maintained via a TTL-bumper.
//!
//! ## Replay-Attack Prevention
//!
//! The `spend` function is the only way to mark a nullifier as used.  It is
//! called *after* a successful proof verification and *before* any transfer is
//! executed.  This ordering is enforced by the contract's state machine in
//! `lib.rs`.
//!
//! ## Nullifier Hash Format Validation
//!
//! Issue #323 requires strict format validation:
//! * The zero nullifier `0x00…00` must be rejected (circuit sentinel).
//! * The all-ones nullifier `0xFF…FF` must be rejected (another sentinel).
//! * The nullifier must not equal the BLS12-381 scalar field order (r).
//! * The nullifier must not equal the BN254 scalar field order (r).
//!
//! These checks are cheap (32-byte comparisons) and prevent circuits with
//! subtle bugs from producing trivially-invalid nullifiers that would pass
//! field-element checks but are not true circuit outputs.
//!
//! ## TTL Policy
//!
//! | TTL Setting         | Ledgers      | Approx. Time (5 s/ledger) |
//! |---------------------|-------------|---------------------------|
//! | Minimum TTL         | 100          | 8.3 minutes               |
//! | Extension threshold | 2 000 000    | ~116 days                 |
//! | Extension target    | 18 460 800   | ~3.0 years                |
//!
//! Nullifiers are extended at write time.  A permissionless TTL-bumper service
//! can call `batch_extend_ttl` to renew entries approaching expiry.

use soroban_sdk::{symbol_short, BytesN, Env, Symbol};

use crate::errors::ZkError;

// ─── Storage Key ───────────────────────────────────────────────────────────

/// Namespace prefix for all nullifier entries.  Using a short symbol keeps
/// ledger keys compact, reducing storage fees.
const NULLIFIER_NS: Symbol = symbol_short!("nf");

/// BLS12-381 scalar field order r (the canonical nullifier domain).
///
/// A nullifier equal to r would be a trivially-invalid field element.
/// We reject it explicitly to guard against buggy circuit outputs.
///
/// r = 0x73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001
const BLS12_381_FR_ORDER: [u8; 32] = [
    0x73, 0xed, 0xa7, 0x53, 0x29, 0x9d, 0x7d, 0x48,
    0x33, 0x39, 0xd8, 0x08, 0x09, 0xa1, 0xd8, 0x05,
    0x53, 0xbd, 0xa4, 0x02, 0xff, 0xfe, 0x5b, 0xfe,
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
];

/// BN254 scalar field order r (retained for backward compatibility).
///
/// r = 0x30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001
const BN254_FR_ORDER: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29,
    0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91,
    0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// All-ones 32-byte sentinel.  A valid ZK circuit should never produce this.
const ALL_ONES_NULLIFIER: [u8; 32] = [0xFF_u8; 32];

// ─── TTL Policy Constants ──────────────────────────────────────────────────

/// Extend TTL unconditionally when bumping (threshold = 0 means always extend).
const TTL_EXTEND_THRESHOLD: u32 = 0;

/// Target TTL for nullifier entries: approximately 3 years.
///
/// At 5 seconds per ledger:
///   18 460 800 ledgers × 5 s = 92 304 000 s ≈ 2.93 years
///
/// This is the maximum TTL allowed by Soroban Protocol 21+.
const TTL_EXTEND_TO: u32 = 18_460_800;

/// Minimum TTL we accept before initiating a proactive extension.
///
/// When an existing nullifier entry has fewer than this many ledgers remaining,
/// a TTL-bumper service should call `extend_nullifier_ttl` (or `batch_extend_ttl`).
/// Set to ~116 days as an early-warning threshold.
pub const TTL_WARNING_THRESHOLD: u32 = 2_000_000;

// ─── Internal Helpers ─────────────────────────────────────────────────────

/// Build the composite ledger key for a nullifier.
///
/// We store `(NULLIFIER_NS, hash)` as the key so nullifier entries are
/// namespaced away from all other contract storage keys (admin, vk, root,
/// pool counter).
#[inline]
fn nullifier_key(hash: &BytesN<32>) -> (Symbol, BytesN<32>) {
    (NULLIFIER_NS, hash.clone())
}

/// Validate the format of a nullifier hash.
///
/// Enforces the following rules required by issue #323:
///
/// 1. **Non-zero**: the all-zero nullifier `0x00…00` is a circuit sentinel.
/// 2. **Non-sentinel**: the all-`0xFF` nullifier is another known invalid value.
/// 3. **Not a field order**: a hash equal to the BLS12-381 or BN254 Fr order
///    is a degenerate value that would fail field-element checks in the circuit.
///
/// Returns `Ok(())` if the format is valid, or a specific error otherwise.
fn validate_nullifier_format(env: &Env, hash: &BytesN<32>) -> Result<(), ZkError> {
    let arr: [u8; 32] = hash.to_array();

    // Rule 1: Non-zero sentinel check
    let zero = BytesN::from_array(env, &[0u8; 32]);
    if hash == &zero {
        return Err(ZkError::NullifierIsZero);
    }

    // Rule 2: All-ones sentinel check
    if arr == ALL_ONES_NULLIFIER {
        return Err(ZkError::NullifierIsZero); // reuse sentinel error code
    }

    // Rule 3: BLS12-381 Fr order check
    if arr == BLS12_381_FR_ORDER {
        return Err(ZkError::InvalidFieldElement);
    }

    // Rule 3b: BN254 Fr order check (backward compat)
    if arr == BN254_FR_ORDER {
        return Err(ZkError::InvalidFieldElement);
    }

    Ok(())
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Check whether `hash` has already been spent.
///
/// Returns `true` if the nullifier is present in Persistent storage.
///
/// This is a read-only, gas-cheap operation designed to be called as an
/// early-exit gate before the expensive ZK proof verification.
pub fn is_spent(env: &Env, hash: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&nullifier_key(hash))
}

/// Mark `hash` as spent by writing the current ledger sequence number.
///
/// ## Validation
///
/// Before writing, the nullifier is validated:
/// * Must not be all-zero (`ZkError::NullifierIsZero`).
/// * Must not be the all-`0xFF` sentinel (`ZkError::NullifierIsZero`).
/// * Must not equal the BLS12-381 or BN254 Fr order (`ZkError::InvalidFieldElement`).
/// * Must not already be spent (`ZkError::NullifierAlreadySpent`).
///
/// ## Storage
///
/// Writes `(ledger_sequence: u32)` to Persistent storage under the key
/// `(NULLIFIER_NS, hash)`.  Immediately extends the TTL to `TTL_EXTEND_TO`
/// ledgers so the entry survives indefinitely.
///
/// ## Returns
///
/// `Ok(())` if the nullifier was successfully marked as spent.
///
/// ## Errors
///
/// * [`ZkError::NullifierIsZero`] – if `hash` is all-zero or all-`0xFF`.
/// * [`ZkError::InvalidFieldElement`] – if `hash` equals a field order.
/// * [`ZkError::NullifierAlreadySpent`] – if `hash` was already recorded.
///
/// ## Panics
///
/// Does not panic.  All failure paths return `Err`.
pub fn spend(env: &Env, hash: &BytesN<32>) -> Result<(), ZkError> {
    // Format validation (cheap, before storage lookup)
    validate_nullifier_format(env, hash)?;

    let key = nullifier_key(hash);

    // Double-spend check (after format validation to give consistent ordering
    // of errors: format errors first, then replay errors)
    if env.storage().persistent().has(&key) {
        return Err(ZkError::NullifierAlreadySpent);
    }

    // Record the ledger sequence for auditability
    let ledger_seq: u32 = env.ledger().sequence();
    env.storage().persistent().set(&key, &ledger_seq);

    // Extend the TTL of this entry so it survives indefinitely.
    // We call this unconditionally at write time; the TTL-bumper service
    // handles renewal of older entries.
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_EXTEND_THRESHOLD, TTL_EXTEND_TO);

    Ok(())
}

/// Return the ledger sequence at which `hash` was spent, or `None` if not spent.
///
/// Useful for auditors and explorers to determine when a nullifier was consumed.
pub fn spent_at(env: &Env, hash: &BytesN<32>) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<(Symbol, BytesN<32>), u32>(&nullifier_key(hash))
}

/// Extend the TTL of an existing nullifier entry.
///
/// This is a no-op if the nullifier entry does not exist in storage (i.e.
/// the nullifier has not been spent yet).  It should be called periodically
/// by a permissionless TTL-bumper service.
///
/// ## TTL Parameters
///
/// * `threshold_ledgers` = `TTL_EXTEND_THRESHOLD` (0 → always extend)
/// * `extend_to_ledgers` = `TTL_EXTEND_TO` (~3 years at 5 s/ledger)
pub fn extend_nullifier_ttl(env: &Env, hash: &BytesN<32>) {
    let key = nullifier_key(hash);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_EXTEND_THRESHOLD, TTL_EXTEND_TO);
    }
}

/// Batch-extend TTLs for a list of nullifier hashes.
///
/// Each entry's TTL is extended to `TTL_EXTEND_TO` ledgers.  Entries that do
/// not exist in storage (not yet spent) are silently skipped.
///
/// This function is designed for maintenance transactions that bump many
/// nullifiers efficiently in a single call.
///
/// ## Gas Notes
///
/// Each TTL extension costs approximately `STORAGE_ENTRY_INSTR` instructions.
/// Callers should keep batch sizes small (≤ 50) to stay within the per-
/// transaction budget.
pub fn batch_extend_ttl(env: &Env, hashes: &soroban_sdk::Vec<BytesN<32>>) {
    for hash in hashes.iter() {
        extend_nullifier_ttl(env, &hash);
    }
}

/// Batch-validate a list of nullifiers without spending them.
///
/// Validates the format of each nullifier and checks that none have already
/// been spent.  Returns `Ok(())` if all are valid and unspent.
///
/// This is useful for a relayer that wants to pre-validate a batch of proofs
/// before submitting them (e.g. to reject known-spent nullifiers cheaply).
///
/// ## Returns
///
/// `Ok(())` if all nullifiers are valid and unspent.
/// `Err(ZkError::NullifierAlreadySpent)` if any nullifier has already been spent.
/// `Err(ZkError::NullifierIsZero)` or `Err(ZkError::InvalidFieldElement)` if
/// any nullifier fails format validation.
pub fn batch_validate_unspent(
    env: &Env,
    hashes: &soroban_sdk::Vec<BytesN<32>>,
) -> Result<(), ZkError> {
    for hash in hashes.iter() {
        // Format check first
        validate_nullifier_format(env, &hash)?;
        // Then spent check
        if is_spent(env, &hash) {
            return Err(ZkError::NullifierAlreadySpent);
        }
    }
    Ok(())
}

// ─── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Ledger as _, LedgerInfo};
    use soroban_sdk::{BytesN, Env};

    fn make_hash(env: &Env, byte: u8) -> BytesN<32> {
        BytesN::from_array(env, &[byte; 32])
    }

    fn set_ledger_seq(env: &Env, seq: u32) {
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000,
            protocol_version: 21,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: TTL_EXTEND_TO,
        });
    }

    // ── Format validation tests ──────────────────────────────────────

    #[test]
    fn zero_nullifier_is_rejected() {
        let env = Env::default();
        let zero = BytesN::from_array(&env, &[0u8; 32]);
        assert_eq!(
            validate_nullifier_format(&env, &zero),
            Err(ZkError::NullifierIsZero)
        );
    }

    #[test]
    fn all_ones_nullifier_is_rejected() {
        let env = Env::default();
        let ones = BytesN::from_array(&env, &[0xFF_u8; 32]);
        assert_eq!(
            validate_nullifier_format(&env, &ones),
            Err(ZkError::NullifierIsZero)
        );
    }

    #[test]
    fn bls12_381_order_nullifier_is_rejected() {
        let env = Env::default();
        let order = BytesN::from_array(&env, &BLS12_381_FR_ORDER);
        assert_eq!(
            validate_nullifier_format(&env, &order),
            Err(ZkError::InvalidFieldElement)
        );
    }

    #[test]
    fn bn254_order_nullifier_is_rejected() {
        let env = Env::default();
        let order = BytesN::from_array(&env, &BN254_FR_ORDER);
        assert_eq!(
            validate_nullifier_format(&env, &order),
            Err(ZkError::InvalidFieldElement)
        );
    }

    #[test]
    fn valid_nullifier_passes_format_check() {
        let env = Env::default();
        let hash = make_hash(&env, 0xAB);
        assert!(validate_nullifier_format(&env, &hash).is_ok());
    }

    // ── Spend lifecycle tests ────────────────────────────────────────

    #[test]
    fn spend_valid_nullifier_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger_seq(&env, 100);

        let hash = make_hash(&env, 0x42);
        assert!(spend(&env, &hash).is_ok());
        assert!(is_spent(&env, &hash));
    }

    #[test]
    fn double_spend_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger_seq(&env, 200);

        let hash = make_hash(&env, 0x55);
        spend(&env, &hash).expect("first spend should succeed");
        assert_eq!(spend(&env, &hash), Err(ZkError::NullifierAlreadySpent));
    }

    #[test]
    fn spend_zero_is_rejected_before_storage_lookup() {
        let env = Env::default();
        let zero = BytesN::from_array(&env, &[0u8; 32]);
        assert_eq!(spend(&env, &zero), Err(ZkError::NullifierIsZero));
        // Must not be stored
        assert!(!is_spent(&env, &zero));
    }

    #[test]
    fn spent_at_returns_correct_ledger_sequence() {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger_seq(&env, 777);

        let hash = make_hash(&env, 0x77);
        spend(&env, &hash).expect("spend OK");

        let seq = spent_at(&env, &hash);
        assert_eq!(seq, Some(777));
    }

    #[test]
    fn spent_at_returns_none_for_unspent() {
        let env = Env::default();
        let hash = make_hash(&env, 0x01);
        assert!(spent_at(&env, &hash).is_none());
    }

    // ── TTL management tests ─────────────────────────────────────────

    #[test]
    fn extend_ttl_on_unspent_is_noop() {
        let env = Env::default();
        env.mock_all_auths();
        let hash = make_hash(&env, 0x99);
        // Should not panic
        extend_nullifier_ttl(&env, &hash);
        assert!(!is_spent(&env, &hash));
    }

    #[test]
    fn batch_extend_ttl_processes_all() {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger_seq(&env, 300);

        let mut hashes = soroban_sdk::Vec::new(&env);
        for b in 1u8..=5u8 {
            let h = make_hash(&env, b);
            spend(&env, &h).expect("spend OK");
            hashes.push_back(h);
        }
        // Batch extend should not panic
        batch_extend_ttl(&env, &hashes);
    }

    // ── Batch validation tests ───────────────────────────────────────

    #[test]
    fn batch_validate_all_unspent_succeeds() {
        let env = Env::default();
        let mut hashes = soroban_sdk::Vec::new(&env);
        for b in [0x10u8, 0x20, 0x30] {
            hashes.push_back(make_hash(&env, b));
        }
        assert!(batch_validate_unspent(&env, &hashes).is_ok());
    }

    #[test]
    fn batch_validate_fails_if_one_spent() {
        let env = Env::default();
        env.mock_all_auths();
        set_ledger_seq(&env, 400);

        let h1 = make_hash(&env, 0xA1);
        let h2 = make_hash(&env, 0xA2);
        let h3 = make_hash(&env, 0xA3);

        // Spend h2
        spend(&env, &h2).expect("spend h2 OK");

        let mut hashes = soroban_sdk::Vec::new(&env);
        hashes.push_back(h1);
        hashes.push_back(h2);
        hashes.push_back(h3);

        assert_eq!(
            batch_validate_unspent(&env, &hashes),
            Err(ZkError::NullifierAlreadySpent)
        );
    }

    #[test]
    fn batch_validate_fails_on_zero_hash() {
        let env = Env::default();
        let zero = BytesN::from_array(&env, &[0u8; 32]);
        let valid = make_hash(&env, 0x01);

        let mut hashes = soroban_sdk::Vec::new(&env);
        hashes.push_back(valid);
        hashes.push_back(zero);

        assert_eq!(
            batch_validate_unspent(&env, &hashes),
            Err(ZkError::NullifierIsZero)
        );
    }

    // ── TTL constant sanity checks ───────────────────────────────────

    #[test]
    fn ttl_extend_to_is_approximately_3_years() {
        // At 5 seconds/ledger: 18_460_800 × 5 = 92_304_000 s ≈ 2.93 years
        let seconds = TTL_EXTEND_TO as u64 * 5;
        let years = seconds / (365 * 24 * 3600);
        assert!(years >= 2 && years <= 4, "TTL should be ~3 years, got {}", years);
    }

    #[test]
    fn ttl_warning_threshold_less_than_extend_to() {
        assert!(TTL_WARNING_THRESHOLD < TTL_EXTEND_TO);
    }
}
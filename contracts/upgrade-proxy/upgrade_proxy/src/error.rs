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
//! Error codes returned by the Upgrade Proxy contract.
//!
//! Every variant is assigned a fixed `u32` discriminant.  These codes are
//! surfaced directly to callers via the Soroban host error mechanism and
//! should never be re-ordered or re-numbered once the contract is deployed.

use soroban_sdk::contracterror;

/// All error conditions that the Upgrade Proxy may return.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProxyError {
    /// `initialize` was called on an already-initialised contract.
    AlreadyInitialized = 1,

    /// A function that requires prior initialisation was called before
    /// `initialize`.
    NotInitialized = 2,

    /// The caller is not permitted to perform the requested operation.
    Unauthorized = 3,

    /// A new upgrade was proposed while one is already pending.  Call
    /// `abort_upgrade` first to replace an existing proposal.
    UpgradeAlreadyPending = 4,

    /// An operation that requires a pending upgrade (e.g. `execute_upgrade`
    /// or `abort_upgrade`) was called when no upgrade is queued.
    NoPendingUpgrade = 5,

    /// `execute_upgrade` was called before the mandatory 7-day window elapsed.
    TimelockNotElapsed = 6,

    /// Arithmetic overflow detected (should never occur in practice but is
    /// included as a safety net).
    ArithmeticOverflow = 7,
}

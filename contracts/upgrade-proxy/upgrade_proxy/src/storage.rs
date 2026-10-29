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
//! Storage key definitions for the Upgrade Proxy contract.
//!
//! Every key is prefixed with `UpgradeProxy` to guarantee it cannot collide
//! with keys from a future contract that might be deployed behind this proxy.
//! Soroban serialises fieldless `#[contracttype]` enum variants by name, so
//! the collision-avoidance guarantee holds as long as the host contract never
//! declares a storage key whose name begins with `UpgradeProxy`.

use soroban_sdk::contracttype;

/// Storage keys used by the Upgrade Proxy governance state machine.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// The privileged admin address — the only address permitted to propose
    /// or execute an upgrade.  Typically a DAO multi-sig.
    UpgradeProxyAdmin,

    /// The DAO emergency address — permitted to call `abort_upgrade` in
    /// addition to the admin.  Should be an n-of-m multi-sig with a
    /// different key-set from the admin so a single compromised key cannot
    /// both propose *and* abort.
    UpgradeProxyDaoCouncil,

    /// The single pending upgrade slot.  At most one upgrade may be in-flight
    /// at a time; a new proposal must first abort the existing one.
    UpgradeProxyPending,
}

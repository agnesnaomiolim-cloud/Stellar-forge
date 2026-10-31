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
//! Declarative **RBAC modifier guards** for easy integration into Soroban
//! contracts.
//!
//! Existing and future contracts store a [`RbacState`][crate::RbacState] and
//! simply annotate privileged entrypoints:
//!
//! ```ignore
//! #[contractimpl]
//! impl Payouts {
//!     pub fn pay(env: Env, to: Address, amount: i128, caller: Address) -> Result<i128, Error> {
//!         stellar_rbac_manager::require_role!(rbac, caller, Role::Operator);
//!         // ... money movement only reachable by Operators
//!     }
//! }
//! ```
//!
//! The guard is lifted from the same O(1) membership check the library uses,
//! so it inherits the low instruction budget.

/// Guards a function by requiring `$caller` to hold `$role`, early-returning a
/// [`RbacError::MissingRole`][crate::RbacError::MissingRole] otherwise.
///
/// Expands to a `?`-based statement, so it must be used in a function
/// returning `Result<_, stellar_rbac_manager::RbacError>` (or a supertype).
#[macro_export]
macro_rules! require_role {
    ($rbac:expr, $caller:expr, $role:expr) => {
        $rbac.require_role($caller, $role)?
    };
}

/// Pure boolean test of role membership — the lowest-cost guard (no error
/// construction, just the O(1) lookup).
#[macro_export]
macro_rules! check_role {
    ($rbac:expr, $caller:expr, $role:expr) => {
        $rbac.has_role($caller, $role)
    };
}
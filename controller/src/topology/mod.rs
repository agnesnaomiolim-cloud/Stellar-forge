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
//! Dynamic node topology spread constraint enforcement.
//!
//! This module inspects the availability-zone topology of the active cluster
//! and generates Kubernetes `topologySpreadConstraints` (and a matching
//! pod anti-affinity fallback) for node StatefulSet manifests.
//!
//! - [`rules`] builds the spread/anti-affinity rules from observed zone data.
//! - [`enforcer`] is the mutating controller that inspects node labels and
//!   injects the generated rules into StatefulSet manifests.

pub mod enforcer;
pub mod rules;

pub use enforcer::{TopologyEnforcer, TopologyError};
pub use rules::{
    build_anti_affinity, build_topology_spread_constraints, EnforcementMode, TopologyRuleSet,
    ZoneCapacity,
};
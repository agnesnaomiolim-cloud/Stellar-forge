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
//! Network traffic shaping and CNI plugin integration module.
///
/// This module exposes the zero-downtime egress traffic shaping controller.
/// It dynamically applies Linux Traffic Control (tc) bandwidth limits to pods
/// via Calico or Cilium CNI Custom Resources, and prioritizes intra-cluster
/// mTLS traffic over public-facing ingress.

pub mod cni;
pub mod shaper;

pub use cni::{CniPlugin, CniPluginKind, CniError, PodSelector, NetworkPolicySpec};
pub use shaper::{TrafficShaper, ShaperConfig, ShaperError, EgressLimit, PriorityClass, ShaperStatus};

/// Re-export the high-level controller that ties together CNI discovery and
/// traffic shaping for the network subsystem.
pub use shaper::TrafficShapingController;

/// Convenience alias for the controller type most callers will use.
pub type Controller = shaper::TrafficShapingController;
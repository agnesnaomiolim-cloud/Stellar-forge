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

//! Topology spread constraint rule generation for Stellar node StatefulSets.
//!
//! This module defines the rule types and the generation logic used by
//! [`super::enforcer`] to build `topologySpreadConstraints` and pod
//! anti-affinity rules that prevent multiple Stellar node pods from landing on
//! the same physical host or availability zone.
//!
//! # Rule hierarchy
//!
//! | Mode             | When applied                            | k8s semantics           |
//! |------------------|-----------------------------------------|-------------------------|
//! | `HardMultiZone`  | ≥ 3 zones **and** enough node capacity  | `DoNotSchedule`         |
//! | `SoftSingleZone` | 1–2 zones, dev clusters, low capacity   | `ScheduleAnyway`        |
//!
//! `HardMultiZone` additionally emits `requiredDuringScheduling…` host
//! anti-affinity, while `SoftSingleZone` emits `preferredDuringScheduling…` so
//! that single-zone local development clusters keep scheduling instead of
//! dead-locking on an unsatisfiable rule.
//!
//! # Capacity gate
//!
//! Hard rules are only applied after node capacity across the availability
//! zones has been recognised: [`ClusterTopology::has_multi_zone`] requires at
//! least [`MIN_ZONES_FOR_HARD_SPREAD`] zones that each hold a schedulable node,
//! and [`build_rule_set`] additionally requires [`ClusterTopology::has_capacity_for`]
//! the requested replica count.  Anything below that falls back to soft rules.
//!
//! # Label keys
//!
//! The well-known Kubernetes topology labels are used:
//!
//! - Zone:  `topology.kubernetes.io/zone`
//! - Host:  `kubernetes.io/hostname`

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Public constants ──────────────────────────────────────────────────────────

/// Well-known Kubernetes label that identifies the failure-domain zone.
pub const TOPOLOGY_ZONE_KEY: &str = "topology.kubernetes.io/zone";

/// Well-known Kubernetes label that identifies the individual node hostname.
pub const TOPOLOGY_HOST_KEY: &str = "kubernetes.io/hostname";

/// Minimum number of distinct availability zones required before hard zone
/// spread rules are enforced.  Clusters with fewer zones (for example a local
/// single-zone development cluster) fall back to soft rules.
pub const MIN_ZONES_FOR_HARD_SPREAD: usize = 3;

// ── Zone detection ────────────────────────────────────────────────────────────

/// Detected topology of the cluster, derived by inspecting live node labels.
///
/// The struct stores the schedulable node *capacity* of every zone so that the
/// enforcer can decide between hard and soft enforcement based on how much
/// capacity actually exists in each availability zone — not merely on how many
/// distinct zone labels were seen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterTopology {
    /// Schedulable node count per zone label value (sorted by zone name).
    zone_capacity: BTreeMap<String, usize>,
    /// Schedulable nodes that carry no `topology.kubernetes.io/zone` label.
    unlabeled_nodes: usize,
    /// Total number of schedulable nodes (labelled + unlabelled).
    node_count: usize,
}

impl ClusterTopology {
    /// Build a topology from a per-zone capacity map plus the number of
    /// schedulable nodes that carry no zone label.
    ///
    /// `node_count` is derived as the sum of every zone capacity plus the
    /// unlabelled node count, so the invariant `node_count == sum(zone) +
    /// unlabeled` always holds.
    pub fn new(zone_capacity: BTreeMap<String, usize>, unlabeled_nodes: usize) -> Self {
        let node_count = zone_capacity.values().sum::<usize>() + unlabeled_nodes;
        Self {
            zone_capacity,
            unlabeled_nodes,
            node_count,
        }
    }

    /// Distinct zone names with at least one schedulable node, sorted.
    pub fn zones(&self) -> Vec<String> {
        self.zone_capacity.keys().cloned().collect()
    }

    /// Number of distinct availability zones holding schedulable nodes.
    pub fn zone_count(&self) -> usize {
        self.zone_capacity.len()
    }

    /// Schedulable node capacity inside the given zone (`0` when unknown).
    pub fn capacity_in(&self, zone: &str) -> usize {
        self.zone_capacity.get(zone).copied().unwrap_or(0)
    }

    /// Immutable view of the per-zone schedulable capacity map.
    pub fn zone_capacity(&self) -> &BTreeMap<String, usize> {
        &self.zone_capacity
    }

    /// Smallest per-zone capacity across all discovered zones (`0` if no zone
    /// was discovered).
    pub fn min_zone_capacity(&self) -> usize {
        self.zone_capacity.values().copied().min().unwrap_or(0)
    }

    /// Total number of schedulable nodes.
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Number of schedulable nodes without a zone label.
    pub fn unlabeled_node_count(&self) -> usize {
        self.unlabeled_nodes
    }

    /// `true` when there is enough zone capacity to enforce hard spread rules:
    /// at least [`MIN_ZONES_FOR_HARD_SPREAD`] zones and every zone holding at
    /// least one schedulable node.
    pub fn has_multi_zone(&self) -> bool {
        self.zone_count() >= MIN_ZONES_FOR_HARD_SPREAD
            && self.zone_capacity.values().all(|capacity| *capacity > 0)
    }

    /// `true` for single-zone setups (local dev / staging clusters) where hard
    /// zone spread would be unsatisfiable.
    pub fn is_single_zone(&self) -> bool {
        self.zone_count() <= 1
    }

    /// `true` when the schedulable nodes can host `replicas` pods.  Hard
    /// anti-affinity requires one node per replica, so the enforcer refuses to
    /// emit hard rules when the cluster is too small.
    pub fn has_capacity_for(&self, replicas: usize) -> bool {
        self.node_count >= replicas
    }
}

// ── Spread-constraint unsatisfiable action ────────────────────────────────────

/// Maps to the Kubernetes `whenUnsatisfiable` field of a
/// `TopologySpreadConstraint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhenUnsatisfiable {
    /// Fail scheduling if the spread cannot be satisfied (hard rule).
    DoNotSchedule,
    /// Place the pod anywhere but record the violation (soft rule).
    ScheduleAnyway,
}

impl WhenUnsatisfiable {
    /// The wire value Kubernetes expects for this variant.
    pub fn as_str(self) -> &'static str {
        match self {
            WhenUnsatisfiable::DoNotSchedule => "DoNotSchedule",
            WhenUnsatisfiable::ScheduleAnyway => "ScheduleAnyway",
        }
    }
}

impl std::fmt::Display for WhenUnsatisfiable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Manifest value types ──────────────────────────────────────────────────────
// The controller crate intentionally avoids the heavy `k8s-openapi` dependency
// (see the crate docs), so the manifest fragments are modelled here with
// serde types that serialize to the exact Kubernetes field names.

/// A `labelSelector` fragment as used by spread constraints and affinity terms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    /// Pod labels a rule matches against.
    pub match_labels: BTreeMap<String, String>,
}

impl LabelSelector {
    /// Wrap a plain label map into a `labelSelector`.
    pub fn from_labels(match_labels: BTreeMap<String, String>) -> Self {
        Self { match_labels }
    }
}

/// A single `spec.template.spec.topologySpreadConstraints[]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologySpreadConstraint {
    /// Maximum skew between the most and least loaded topology domain.
    pub max_skew: i32,
    /// Label key whose values define the topology domains.
    pub topology_key: String,
    /// Scheduling action when the constraint cannot be satisfied.
    pub when_unsatisfiable: WhenUnsatisfiable,
    /// Pod label selector used to count existing pods in each domain.
    pub label_selector: LabelSelector,
}

impl TopologySpreadConstraint {
    /// Build a zone-spread constraint for the given pod selector labels.
    pub fn zone_spread(
        selector_labels: BTreeMap<String, String>,
        when_unsatisfiable: WhenUnsatisfiable,
    ) -> Self {
        Self {
            max_skew: 1,
            topology_key: TOPOLOGY_ZONE_KEY.to_string(),
            when_unsatisfiable,
            label_selector: LabelSelector::from_labels(selector_labels),
        }
    }

    /// Build a per-host spread constraint for the given pod selector labels.
    pub fn host_spread(
        selector_labels: BTreeMap<String, String>,
        when_unsatisfiable: WhenUnsatisfiable,
    ) -> Self {
        Self {
            max_skew: 1,
            topology_key: TOPOLOGY_HOST_KEY.to_string(),
            when_unsatisfiable,
            label_selector: LabelSelector::from_labels(selector_labels),
        }
    }
}

/// A `podAffinityTerm` fragment (shared by hard and soft anti-affinity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodAffinityTerm {
    /// Pod labels the term matches against.
    pub label_selector: LabelSelector,
    /// Domain the anti-affinity is evaluated on.
    pub topology_key: String,
    /// Optional namespace scope; omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespaces: Option<Vec<String>>,
}

/// A weighted `podAffinityTerm` used by soft (preferred) anti-affinity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeightedPodAffinityTerm {
    /// Scheduling weight (1–100) attached to the term.
    pub weight: i32,
    /// The term being weighted.
    pub pod_affinity_term: PodAffinityTerm,
}

/// The `spec.template.spec.affinity.podAntiAffinity` fragment.
///
/// Exactly one of the two fields is populated: hard rules populate
/// `requiredDuringSchedulingIgnoredDuringExecution`, soft rules populate
/// `preferredDuringSchedulingIgnoredDuringExecution`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodAntiAffinity {
    /// Hard rules that block scheduling when unsatisfiable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_during_scheduling_ignored_during_execution: Option<Vec<PodAffinityTerm>>,
    /// Soft rules that only bias scheduling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_during_scheduling_ignored_during_execution: Option<Vec<WeightedPodAffinityTerm>>,
}

// ── Anti-affinity helpers ─────────────────────────────────────────────────────

/// Generate a strict (required) `podAntiAffinity` rule that prevents two pods
/// with `selector_labels` from being co-located on the same host.
pub fn hard_host_anti_affinity(
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
) -> PodAntiAffinity {
    let term = host_affinity_term(selector_labels, namespace);
    PodAntiAffinity {
        required_during_scheduling_ignored_during_execution: Some(vec![term]),
        preferred_during_scheduling_ignored_during_execution: None,
    }
}

/// Generate a soft (preferred) `podAntiAffinity` rule that tries to spread pods
/// across hosts but never blocks scheduling when no suitable host is found —
/// the fallback used on single-zone local development clusters.
pub fn soft_host_anti_affinity(
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
    weight: i32,
) -> PodAntiAffinity {
    let term = host_affinity_term(selector_labels, namespace);
    PodAntiAffinity {
        required_during_scheduling_ignored_during_execution: None,
        preferred_during_scheduling_ignored_during_execution: Some(vec![WeightedPodAffinityTerm {
            weight,
            pod_affinity_term: term,
        }]),
    }
}

fn host_affinity_term(selector_labels: BTreeMap<String, String>, namespace: Option<&str>) -> PodAffinityTerm {
    PodAffinityTerm {
        label_selector: LabelSelector::from_labels(selector_labels),
        topology_key: TOPOLOGY_HOST_KEY.to_string(),
        namespaces: namespace.map(|ns| vec![ns.to_string()]),
    }
}

// ── Rule set ──────────────────────────────────────────────────────────────────

/// Which enforcement mode is active for a given cluster topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopologyMode {
    /// Hard zone spread + hard host anti-affinity (≥ 3 zones, enough capacity).
    HardMultiZone,
    /// Soft host spread + soft host anti-affinity (< 3 zones, dev clusters, or
    /// not enough schedulable node capacity).
    SoftSingleZone,
}

impl std::fmt::Display for TopologyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyMode::HardMultiZone => write!(f, "HardMultiZone"),
            TopologyMode::SoftSingleZone => write!(f, "SoftSingleZone"),
        }
    }
}

/// The complete set of topology rules to inject into a StatefulSet pod template.
#[derive(Debug, Clone, PartialEq)]
pub struct TopologyRuleSet {
    /// `topologySpreadConstraints` to inject.
    pub spread_constraints: Vec<TopologySpreadConstraint>,
    /// `podAntiAffinity` to inject.
    pub anti_affinity: PodAntiAffinity,
    /// Which mode was chosen for the detected topology.
    pub mode: TopologyMode,
}

/// Build a [`TopologyRuleSet`] appropriate for the given cluster topology.
///
/// Hard (`DoNotSchedule`) rules are emitted only when **both** gates pass:
///
/// 1. [`ClusterTopology::has_multi_zone`] — at least
///    [`MIN_ZONES_FOR_HARD_SPREAD`] zones each holding schedulable capacity;
/// 2. [`ClusterTopology::has_capacity_for`] — enough schedulable nodes for
///    `replicas` pods under required host anti-affinity.
///
/// Otherwise the rule set falls back to soft (`ScheduleAnyway`) rules so that
/// single-zone local development clusters keep scheduling.
///
/// # Arguments
///
/// * `topology`        – detected cluster topology (zone capacity + nodes)
/// * `selector_labels` – pod labels used for matching in spread constraints
/// * `namespace`       – optional namespace for anti-affinity scoping
/// * `replicas`        – replica count of the StatefulSet being enforced
pub fn build_rule_set(
    topology: &ClusterTopology,
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
    replicas: usize,
) -> TopologyRuleSet {
    let hard = topology.has_multi_zone() && topology.has_capacity_for(replicas);

    if hard {
        // ── Hard mode: 3+ zones with enough schedulable capacity ──────────
        TopologyRuleSet {
            spread_constraints: vec![
                TopologySpreadConstraint::zone_spread(
                    selector_labels.clone(),
                    WhenUnsatisfiable::DoNotSchedule,
                ),
                TopologySpreadConstraint::host_spread(
                    selector_labels.clone(),
                    WhenUnsatisfiable::DoNotSchedule,
                ),
            ],
            anti_affinity: hard_host_anti_affinity(selector_labels, namespace),
            mode: TopologyMode::HardMultiZone,
        }
    } else {
        // ── Soft mode: single-zone / low-capacity fallback ─────────────────
        TopologyRuleSet {
            spread_constraints: vec![TopologySpreadConstraint::host_spread(
                selector_labels.clone(),
                WhenUnsatisfiable::ScheduleAnyway,
            )],
            anti_affinity: soft_host_anti_affinity(selector_labels, namespace, 100),
            mode: TopologyMode::SoftSingleZone,
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_labels() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        );
        m.insert(
            "app.kubernetes.io/component".to_string(),
            "validator".to_string(),
        );
        m
    }

    fn three_zone_topology() -> ClusterTopology {
        let mut capacity = BTreeMap::new();
        capacity.insert("us-east-1a".to_string(), 3);
        capacity.insert("us-east-1b".to_string(), 3);
        capacity.insert("us-east-1c".to_string(), 3);
        ClusterTopology::new(capacity, 0)
    }

    fn single_zone_topology() -> ClusterTopology {
        let mut capacity = BTreeMap::new();
        capacity.insert("local".to_string(), 3);
        ClusterTopology::new(capacity, 0)
    }

    // ── ClusterTopology helpers ───────────────────────────────────────────

    #[test]
    fn test_new_sums_capacity_into_node_count() {
        let t = three_zone_topology();
        assert_eq!(t.node_count(), 9);
        assert_eq!(t.zone_count(), 3);
        assert_eq!(t.min_zone_capacity(), 3);
        assert_eq!(t.unlabeled_node_count(), 0);
    }

    #[test]
    fn test_new_counts_unlabeled_nodes() {
        let t = ClusterTopology::new(BTreeMap::new(), 2);
        assert_eq!(t.node_count(), 2);
        assert_eq!(t.zone_count(), 0);
        assert!(t.zones().is_empty());
    }

    #[test]
    fn test_zones_are_sorted() {
        let mut capacity = BTreeMap::new();
        capacity.insert("us-east-1c".to_string(), 1);
        capacity.insert("us-east-1a".to_string(), 1);
        capacity.insert("us-east-1b".to_string(), 1);
        let t = ClusterTopology::new(capacity, 0);
        assert_eq!(
            t.zones(),
            vec![
                "us-east-1a".to_string(),
                "us-east-1b".to_string(),
                "us-east-1c".to_string()
            ]
        );
    }

    #[test]
    fn test_capacity_in() {
        let t = three_zone_topology();
        assert_eq!(t.capacity_in("us-east-1a"), 3);
        assert_eq!(t.capacity_in("eu-west-1z"), 0);
    }

    #[test]
    fn test_has_multi_zone_true() {
        assert!(three_zone_topology().has_multi_zone());
    }

    #[test]
    fn test_has_multi_zone_false_for_two_zones() {
        let mut capacity = BTreeMap::new();
        capacity.insert("a".to_string(), 2);
        capacity.insert("b".to_string(), 2);
        let t = ClusterTopology::new(capacity, 0);
        assert!(!t.has_multi_zone());
    }

    #[test]
    fn test_has_multi_zone_false_when_a_zone_has_no_capacity() {
        // Three zone labels are visible, but one zone holds no schedulable
        // node — hard rules must not be applied before node capacity across
        // the availability zones is recognised.
        let mut capacity = BTreeMap::new();
        capacity.insert("a".to_string(), 1);
        capacity.insert("b".to_string(), 1);
        capacity.insert("c".to_string(), 0);
        let t = ClusterTopology::new(capacity, 1);
        assert_eq!(t.zone_count(), 3);
        assert_eq!(t.min_zone_capacity(), 0);
        assert!(!t.has_multi_zone());
    }

    #[test]
    fn test_is_single_zone() {
        assert!(single_zone_topology().is_single_zone());
        assert!(ClusterTopology::default().is_single_zone());
    }

    #[test]
    fn test_is_not_single_zone_for_two_zones() {
        let mut capacity = BTreeMap::new();
        capacity.insert("a".to_string(), 2);
        capacity.insert("b".to_string(), 2);
        let t = ClusterTopology::new(capacity, 0);
        assert!(!t.is_single_zone());
    }

    #[test]
    fn test_has_capacity_for_replicas() {
        let t = three_zone_topology();
        assert!(t.has_capacity_for(9));
        assert!(!t.has_capacity_for(10));
    }

    // ── WhenUnsatisfiable ─────────────────────────────────────────────────

    #[test]
    fn test_when_unsatisfiable_as_str() {
        assert_eq!(WhenUnsatisfiable::DoNotSchedule.as_str(), "DoNotSchedule");
        assert_eq!(WhenUnsatisfiable::ScheduleAnyway.as_str(), "ScheduleAnyway");
        assert_eq!(
            WhenUnsatisfiable::DoNotSchedule.to_string(),
            "DoNotSchedule"
        );
    }

    #[test]
    fn test_when_unsatisfiable_serializes_as_wire_value() {
        let value = serde_json::to_value(WhenUnsatisfiable::DoNotSchedule).unwrap();
        assert_eq!(value, serde_json::json!("DoNotSchedule"));
    }

    // ── TopologySpreadConstraint builders ─────────────────────────────────

    #[test]
    fn test_zone_spread_hard() {
        let c = TopologySpreadConstraint::zone_spread(
            sample_labels(),
            WhenUnsatisfiable::DoNotSchedule,
        );
        assert_eq!(c.topology_key, TOPOLOGY_ZONE_KEY);
        assert_eq!(c.when_unsatisfiable, WhenUnsatisfiable::DoNotSchedule);
        assert_eq!(c.max_skew, 1);
        assert_eq!(c.label_selector.match_labels, sample_labels());
    }

    #[test]
    fn test_host_spread_soft() {
        let c = TopologySpreadConstraint::host_spread(
            sample_labels(),
            WhenUnsatisfiable::ScheduleAnyway,
        );
        assert_eq!(c.topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(c.when_unsatisfiable, WhenUnsatisfiable::ScheduleAnyway);
    }

    #[test]
    fn test_spread_constraint_serializes_to_kubernetes_field_names() {
        let c = TopologySpreadConstraint::host_spread(
            sample_labels(),
            WhenUnsatisfiable::ScheduleAnyway,
        );
        let value = serde_json::to_value(&c).unwrap();
        assert_eq!(value["maxSkew"], 1);
        assert_eq!(value["topologyKey"], TOPOLOGY_HOST_KEY);
        assert_eq!(value["whenUnsatisfiable"], "ScheduleAnyway");
        assert_eq!(
            value["labelSelector"]["matchLabels"]["app.kubernetes.io/name"],
            "stellar-node"
        );
    }

    // ── anti-affinity helpers ─────────────────────────────────────────────

    #[test]
    fn test_hard_host_anti_affinity_topology_key() {
        let aff = hard_host_anti_affinity(sample_labels(), Some("stellar"));
        let terms = aff
            .required_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(terms[0].topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(terms[0].namespaces, Some(vec!["stellar".to_string()]));
        assert!(aff.preferred_during_scheduling_ignored_during_execution.is_none());
    }

    #[test]
    fn test_soft_host_anti_affinity_weight() {
        let aff = soft_host_anti_affinity(sample_labels(), None, 100);
        let terms = aff
            .preferred_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(terms[0].weight, 100);
        assert_eq!(terms[0].pod_affinity_term.topology_key, TOPOLOGY_HOST_KEY);
        assert!(terms[0].pod_affinity_term.namespaces.is_none());
        assert!(aff.required_during_scheduling_ignored_during_execution.is_none());
    }

    #[test]
    fn test_anti_affinity_serialization_omits_empty_side() {
        let aff = hard_host_anti_affinity(sample_labels(), None);
        let value = serde_json::to_value(&aff).unwrap();
        assert!(value.get("preferredDuringSchedulingIgnoredDuringExecution").is_none());
        assert!(value.get("requiredDuringSchedulingIgnoredDuringExecution").is_some());
    }

    // ── build_rule_set: multi-zone (hard mode) ────────────────────────────

    #[test]
    fn test_build_rule_set_multi_zone_produces_two_constraints() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"), 6);
        assert_eq!(rs.spread_constraints.len(), 2);
        assert_eq!(rs.mode, TopologyMode::HardMultiZone);
    }

    #[test]
    fn test_build_rule_set_multi_zone_first_constraint_is_zone() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"), 6);
        assert_eq!(rs.spread_constraints[0].topology_key, TOPOLOGY_ZONE_KEY);
        assert_eq!(
            rs.spread_constraints[0].when_unsatisfiable,
            WhenUnsatisfiable::DoNotSchedule
        );
    }

    #[test]
    fn test_build_rule_set_multi_zone_second_constraint_is_host() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"), 6);
        assert_eq!(rs.spread_constraints[1].topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(
            rs.spread_constraints[1].when_unsatisfiable,
            WhenUnsatisfiable::DoNotSchedule
        );
    }

    #[test]
    fn test_build_rule_set_multi_zone_hard_anti_affinity() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"), 6);
        assert!(rs
            .anti_affinity
            .required_during_scheduling_ignored_during_execution
            .as_ref()
            .is_some_and(|terms| !terms.is_empty()));
        assert!(rs
            .anti_affinity
            .preferred_during_scheduling_ignored_during_execution
            .is_none());
    }

    // ── build_rule_set: capacity gate ─────────────────────────────────────

    #[test]
    fn test_build_rule_set_falls_back_to_soft_when_replicas_exceed_capacity() {
        // 3 zones, but only 3 schedulable nodes (one per zone) for 5 replicas:
        // hard required anti-affinity could never be satisfied.
        let mut capacity = BTreeMap::new();
        capacity.insert("us-east-1a".to_string(), 1);
        capacity.insert("us-east-1b".to_string(), 1);
        capacity.insert("us-east-1c".to_string(), 1);
        let t = ClusterTopology::new(capacity, 0);
        let rs = build_rule_set(&t, sample_labels(), Some("stellar"), 5);
        assert_eq!(rs.mode, TopologyMode::SoftSingleZone);
        assert_eq!(
            rs.spread_constraints[0].when_unsatisfiable,
            WhenUnsatisfiable::ScheduleAnyway
        );
    }

    #[test]
    fn test_build_rule_set_hard_mode_when_capacity_matches_replicas() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"), 9);
        assert_eq!(rs.mode, TopologyMode::HardMultiZone);
    }

    // ── build_rule_set: single-zone (soft mode) ───────────────────────────

    #[test]
    fn test_build_rule_set_single_zone_produces_one_constraint() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None, 3);
        assert_eq!(rs.spread_constraints.len(), 1);
        assert_eq!(rs.mode, TopologyMode::SoftSingleZone);
    }

    #[test]
    fn test_build_rule_set_single_zone_constraint_is_host_soft() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None, 3);
        assert_eq!(rs.spread_constraints[0].topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(
            rs.spread_constraints[0].when_unsatisfiable,
            WhenUnsatisfiable::ScheduleAnyway
        );
    }

    #[test]
    fn test_build_rule_set_single_zone_soft_anti_affinity() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None, 3);
        assert!(rs
            .anti_affinity
            .preferred_during_scheduling_ignored_during_execution
            .as_ref()
            .is_some_and(|terms| !terms.is_empty()));
        assert!(rs
            .anti_affinity
            .required_during_scheduling_ignored_during_execution
            .is_none());
    }

    #[test]
    fn test_build_rule_set_two_zones_selects_soft_mode() {
        let mut capacity = BTreeMap::new();
        capacity.insert("a".to_string(), 4);
        capacity.insert("b".to_string(), 4);
        let t = ClusterTopology::new(capacity, 0);
        let rs = build_rule_set(&t, sample_labels(), None, 4);
        assert_eq!(rs.mode, TopologyMode::SoftSingleZone);
    }

    #[test]
    fn test_build_rule_set_no_zones_selects_soft_mode() {
        let t = ClusterTopology::new(BTreeMap::new(), 2);
        let rs = build_rule_set(&t, sample_labels(), None, 2);
        assert_eq!(rs.mode, TopologyMode::SoftSingleZone);
    }

    // ── TopologyMode display ──────────────────────────────────────────────

    #[test]
    fn test_topology_mode_display() {
        assert_eq!(TopologyMode::HardMultiZone.to_string(), "HardMultiZone");
        assert_eq!(TopologyMode::SoftSingleZone.to_string(), "SoftSingleZone");
    }
}

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

//! Dynamic Node Topology Spread Constraint Enforcer.
//!
//! This module implements a mutating controller that:
//!
//! 1. Inspects the active node provider's zone labels through a
//!    [`NodeZoneSource`] (live inventory, fixtures, or local dev clusters).
//! 2. Derives the appropriate [`TopologyRuleSet`] (hard vs. soft) using
//!    [`super::rules`], recognising node capacity per availability zone
//!    before hard rules are applied.
//! 3. Injects `topologySpreadConstraints` and `podAntiAffinity` rules into a
//!    Stellar-node `StatefulSet` manifest ([`enforce_statefulset_manifest`])
//!    or renders the equivalent merge patch ([`build_statefulset_patch`]).
//!
//! # Behaviour
//!
//! | Detected topology                            | Action                                    |
//! |----------------------------------------------|-------------------------------------------|
//! | ≥ 3 zones, capacity for the replica count     | Hard `DoNotSchedule` zone + host spread   |
//! | 1–2 zones (e.g. local dev cluster)            | Soft `ScheduleAnyway` host spread         |
//! | 3+ zones but too few schedulable nodes        | Soft `ScheduleAnyway` host spread         |
//! | No zone labels at all                         | Soft `ScheduleAnyway` host spread         |
//!
//! The controller is idempotent — the desired constraints are recomputed on
//! every call, so re-running enforcement against an already-patched manifest
//! is a no-op.
//!
//! # Related modules
//!
//! - [`super::rules`] — rule generation logic and types

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use thiserror::Error;
use tracing::{debug, info, warn};

use super::rules::{build_rule_set, ClusterTopology, TopologyMode, TopologyRuleSet, TOPOLOGY_ZONE_KEY};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Annotation set on StatefulSets to record the active enforcement mode.
pub const TOPOLOGY_MODE_ANNOTATION: &str = "stellar.org/topology-mode";

/// Annotation set on StatefulSets to record the observed zone count.
pub const TOPOLOGY_ZONES_ANNOTATION: &str = "stellar.org/topology-zone-count";

/// Annotation set on StatefulSets to record the observed schedulable node
/// count that the capacity gate was evaluated against.
pub const TOPOLOGY_NODES_ANNOTATION: &str = "stellar.org/topology-node-count";

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors raised while discovering topology or mutating a manifest.
#[derive(Debug, Error)]
pub enum TopologyError {
    /// The node zone source failed while listing node labels.
    #[error("node zone source failed: {0}")]
    NodeSource(String),
    /// The target manifest is not a usable StatefulSet object.
    #[error("invalid StatefulSet manifest: {0}")]
    InvalidManifest(String),
    /// A rule fragment could not be serialized to JSON.
    #[error("failed to serialize topology rules: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ── Node inventory ────────────────────────────────────────────────────────────

/// Minimal description of a node as needed for topology decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    /// Node name (used for diagnostics only).
    pub name: String,
    /// Value of the `topology.kubernetes.io/zone` label, when present.
    pub zone: Option<String>,
    /// Whether the node reports `Ready`.
    pub ready: bool,
    /// Whether the node is cordoned (`unschedulable: true`).
    pub unschedulable: bool,
}

impl NodeInfo {
    /// A ready, schedulable node with the given zone label.
    pub fn new(name: impl Into<String>, zone: Option<String>) -> Self {
        Self {
            name: name.into(),
            zone,
            ready: true,
            unschedulable: false,
        }
    }

    /// Override readiness / cordoned status.
    pub fn with_status(mut self, ready: bool, unschedulable: bool) -> Self {
        self.ready = ready;
        self.unschedulable = unschedulable;
        self
    }

    /// Build a [`NodeInfo`] from raw node labels, reading the zone from the
    /// well-known [`TOPOLOGY_ZONE_KEY`] label.
    pub fn from_labels(name: impl Into<String>, labels: &BTreeMap<String, String>) -> Self {
        let zone = labels.get(TOPOLOGY_ZONE_KEY).cloned().filter(|z| !z.is_empty());
        Self::new(name, zone)
    }

    /// `true` when the node may receive pods (ready and not cordoned).
    pub fn is_schedulable(&self) -> bool {
        self.ready && !self.unschedulable
    }
}

/// Source of the active node provider's zone labels.
///
/// The enforcer is deliberately decoupled from any specific Kubernetes client:
/// the operator plugs in whatever inventory it has (API list, cached sync, or a
/// local dev-cluster fixture) and the enforcer derives the topology from it.
pub trait NodeZoneSource {
    /// List the zone labels (and scheduling status) of all cluster nodes.
    fn node_zone_labels(&self) -> Result<Vec<NodeInfo>, TopologyError>;
}

/// A static, in-memory node inventory.
///
/// Used by unit tests, manifest-generation tests, and single-zone local
/// development clusters where no live node API is available.
#[derive(Debug, Clone, Default)]
pub struct StaticNodeZoneSource {
    nodes: Vec<NodeInfo>,
}

impl StaticNodeZoneSource {
    /// Wrap an explicit node inventory.
    pub fn new(nodes: Vec<NodeInfo>) -> Self {
        Self { nodes }
    }

    /// Build one schedulable node per zone — the smallest cluster that can
    /// satisfy hard rules.
    pub fn from_zone_labels<I, S>(zones: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::with_nodes_per_zone(zones, 1)
    }

    /// Build `nodes_per_zone` schedulable nodes in each of the given zones.
    pub fn with_nodes_per_zone<I, S>(zones: I, nodes_per_zone: usize) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut nodes = Vec::new();
        for zone in zones {
            let zone = zone.into();
            for idx in 0..nodes_per_zone {
                nodes.push(NodeInfo::new(format!("{zone}-node-{idx}"), Some(zone.clone())));
            }
        }
        Self { nodes }
    }

    /// Build a source from raw `metadata.name → labels` style label maps,
    /// reading the zone label from each map.
    pub fn from_label_maps(maps: &[BTreeMap<String, String>]) -> Self {
        let nodes = maps
            .iter()
            .enumerate()
            .map(|(idx, labels)| NodeInfo::from_labels(format!("node-{idx}"), labels))
            .collect();
        Self { nodes }
    }

    /// Number of nodes in the inventory.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` when the inventory holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl NodeZoneSource for StaticNodeZoneSource {
    fn node_zone_labels(&self) -> Result<Vec<NodeInfo>, TopologyError> {
        Ok(self.nodes.clone())
    }
}

impl NodeZoneSource for Vec<NodeInfo> {
    fn node_zone_labels(&self) -> Result<Vec<NodeInfo>, TopologyError> {
        Ok(self.clone())
    }
}

// ── Topology discovery ────────────────────────────────────────────────────────

/// Discover the cluster topology by inspecting the node provider's zone labels.
///
/// Only nodes that are `Ready` and not cordoned contribute capacity; zoneless
/// nodes are counted separately so callers can reason about unlabelled
/// clusters.
pub fn discover_cluster_topology(
    source: &dyn NodeZoneSource,
) -> Result<ClusterTopology, TopologyError> {
    let nodes = source.node_zone_labels()?;

    let mut zone_capacity: BTreeMap<String, usize> = BTreeMap::new();
    let mut unlabeled_nodes = 0usize;

    for node in &nodes {
        if !node.is_schedulable() {
            debug!(node = %node.name, "skipping non-schedulable node during topology discovery");
            continue;
        }
        match node.zone.as_deref() {
            Some(zone) if !zone.is_empty() => {
                *zone_capacity.entry(zone.to_string()).or_default() += 1
            }
            _ => unlabeled_nodes += 1,
        }
    }

    let topology = ClusterTopology::new(zone_capacity, unlabeled_nodes);
    info!(
        zones = topology.zone_count(),
        nodes = topology.node_count(),
        unlabeled = topology.unlabeled_node_count(),
        "discovered cluster topology"
    );
    Ok(topology)
}

// ── Patch / manifest generation ───────────────────────────────────────────────

/// Build the pod-template merge patch that injects topology rules.
///
/// The returned [`Value`] is suitable for a strategic/merge patch of a
/// StatefulSet and carries the enforcement annotations on `metadata`.
pub fn build_statefulset_patch(
    rule_set: &TopologyRuleSet,
    zone_count: usize,
    node_count: usize,
) -> Result<Value, TopologyError> {
    let spread_constraints: Vec<Value> = rule_set
        .spread_constraints
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    let anti_affinity = serde_json::to_value(&rule_set.anti_affinity)?;

    let mut patch = json!({
        "spec": {
            "template": {
                "spec": {
                    "topologySpreadConstraints": spread_constraints,
                    "affinity": {
                        "podAntiAffinity": anti_affinity
                    }
                }
            }
        }
    });

    let metadata = patch
        .as_object_mut()
        .expect("patch root is an object by construction")
        .entry("metadata".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let annotations = metadata
        .as_object_mut()
        .expect("metadata container is an object by construction")
        .entry("annotations".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let annotations = annotations
        .as_object_mut()
        .expect("annotations container is an object by construction");
    annotations.insert(
        TOPOLOGY_MODE_ANNOTATION.to_string(),
        json!(rule_set.mode.to_string()),
    );
    annotations.insert(
        TOPOLOGY_ZONES_ANNOTATION.to_string(),
        json!(zone_count.to_string()),
    );
    annotations.insert(
        TOPOLOGY_NODES_ANNOTATION.to_string(),
        json!(node_count.to_string()),
    );

    Ok(patch)
}

/// Outcome of a single topology-enforcement run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnforcementResult {
    /// Name of the StatefulSet that was inspected / patched.
    pub statefulset_name: String,
    /// Namespace of the StatefulSet.
    pub namespace: String,
    /// Enforcement mode that was selected.
    pub mode: TopologyMode,
    /// Zones that were detected during this run.
    pub zones: Vec<String>,
    /// Schedulable node capacity observed during this run.
    pub node_count: usize,
    /// Replica count the capacity gate was evaluated against.
    pub replicas: usize,
    /// Whether the manifest was actually mutated (`false` for dry runs and
    /// idempotent no-ops).
    pub patched: bool,
    /// Whether enforcement ran in dry-run mode.
    pub dry_run: bool,
}

/// Mutate a StatefulSet manifest in place, injecting topology spread
/// constraints and pod anti-affinity derived from the live node inventory.
///
/// The manifest's `spec.selector.matchLabels` (falling back to
/// `spec.template.metadata.labels`) are used as the spread/affinity selector,
/// and `spec.replicas` feeds the capacity gate.  Existing annotations are
/// preserved.
pub fn enforce_statefulset_manifest(
    manifest: &mut Value,
    source: &dyn NodeZoneSource,
    dry_run: bool,
) -> Result<EnforcementResult, TopologyError> {
    let kind = manifest
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != "StatefulSet" {
        return Err(TopologyError::InvalidManifest(format!(
            "expected kind StatefulSet, found {kind:?}"
        )));
    }

    let name = manifest
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let namespace = manifest
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_string();
    let replicas = manifest
        .pointer("/spec/replicas")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let selector_labels = selector_labels_from_manifest(manifest);

    let topology = discover_cluster_topology(source)?;
    if topology.has_multi_zone() && !topology.has_capacity_for(replicas) {
        warn!(
            statefulset = %name,
            namespace = %namespace,
            nodes = topology.node_count(),
            replicas,
            "insufficient schedulable node capacity for hard topology rules — falling back to soft rules"
        );
    }

    let rule_set = build_rule_set(&topology, selector_labels, Some(&namespace), replicas);
    info!(
        statefulset = %name,
        namespace = %namespace,
        mode = %rule_set.mode,
        zones = topology.zone_count(),
        nodes = topology.node_count(),
        replicas,
        "enforcing topology rules on StatefulSet"
    );

    if dry_run {
        debug!(statefulset = %name, namespace = %namespace, "dry-run: skipping manifest mutation");
        return Ok(EnforcementResult {
            statefulset_name: name,
            namespace,
            mode: rule_set.mode,
            zones: topology.zones(),
            node_count: topology.node_count(),
            replicas,
            patched: false,
            dry_run: true,
        });
    }

    let before = manifest.clone();
    inject_rules(manifest, &rule_set, &topology)?;
    let patched = *manifest != before;

    Ok(EnforcementResult {
        statefulset_name: name,
        namespace,
        mode: rule_set.mode,
        zones: topology.zones(),
        node_count: topology.node_count(),
        replicas,
        patched,
        dry_run: false,
    })
}

/// Inject the generated rules into the manifest, preserving any unrelated
/// fields (existing annotations, affinity siblings, …).
fn inject_rules(
    manifest: &mut Value,
    rule_set: &TopologyRuleSet,
    topology: &ClusterTopology,
) -> Result<(), TopologyError> {
    if !manifest.is_object() {
        return Err(TopologyError::InvalidManifest(
            "manifest must be a JSON object".to_string(),
        ));
    }

    let spread: Vec<Value> = rule_set
        .spread_constraints
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    let anti_affinity = serde_json::to_value(&rule_set.anti_affinity)?;

    *ensure_object_path(manifest, &["spec", "template", "spec", "topologySpreadConstraints"])? =
        Value::Array(spread);
    *ensure_object_path(manifest, &["spec", "template", "spec", "affinity", "podAntiAffinity"])? =
        anti_affinity;

    let annotations = ensure_object_path(manifest, &["metadata", "annotations"])?
        .as_object_mut()
        .expect("annotation path resolves to an object");
    annotations.insert(
        TOPOLOGY_MODE_ANNOTATION.to_string(),
        json!(rule_set.mode.to_string()),
    );
    annotations.insert(
        TOPOLOGY_ZONES_ANNOTATION.to_string(),
        json!(topology.zone_count().to_string()),
    );
    annotations.insert(
        TOPOLOGY_NODES_ANNOTATION.to_string(),
        json!(topology.node_count().to_string()),
    );

    Ok(())
}

/// Walk `path` from `root`, creating missing JSON objects, and return the
/// value stored at the final segment.
fn ensure_object_path<'a>(
    root: &'a mut Value,
    path: &[&str],
) -> Result<&'a mut Value, TopologyError> {
    let mut cursor = root;
    for segment in path {
        if cursor.is_null() {
            *cursor = Value::Object(Map::new());
        }
        let object = cursor.as_object_mut().ok_or_else(|| {
            TopologyError::InvalidManifest(format!(
                "expected a JSON object at segment {segment:?} of path {path:?}"
            ))
        })?;
        cursor = object
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    Ok(cursor)
}

/// Extract the pod selector labels from a StatefulSet manifest, preferring
/// `spec.selector.matchLabels` and falling back to the pod template labels.
fn selector_labels_from_manifest(manifest: &Value) -> BTreeMap<String, String> {
    for pointer in ["/spec/selector/matchLabels", "/spec/template/metadata/labels"] {
        if let Some(map) = manifest.pointer(pointer).and_then(Value::as_object) {
            let labels: BTreeMap<String, String> = map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect();
            if !labels.is_empty() {
                return labels;
            }
        }
    }
    BTreeMap::new()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::rules::{WhenUnsatisfiable, TOPOLOGY_HOST_KEY, TOPOLOGY_ZONE_KEY};
    use super::*;

    fn three_zone_source() -> StaticNodeZoneSource {
        StaticNodeZoneSource::with_nodes_per_zone(
            ["us-east-1a", "us-east-1b", "us-east-1c"],
            3,
        )
    }

    fn single_zone_source() -> StaticNodeZoneSource {
        StaticNodeZoneSource::with_nodes_per_zone(["local"], 3)
    }

    fn statefulset_manifest(name: &str, namespace: &str, replicas: u64) -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "StatefulSet",
            "metadata": {
                "name": name,
                "namespace": namespace,
                "annotations": { "stellar.org/keep-me": "yes" }
            },
            "spec": {
                "replicas": replicas,
                "selector": { "matchLabels": { "app.kubernetes.io/name": "stellar-node" } },
                "template": {
                    "metadata": { "labels": { "app.kubernetes.io/name": "stellar-node" } },
                    "spec": { "containers": [] }
                }
            }
        })
    }

    fn spread_constraints(manifest: &Value) -> Vec<Value> {
        manifest
            .pointer("/spec/template/spec/topologySpreadConstraints")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    // ── discover_cluster_topology ─────────────────────────────────────────

    #[test]
    fn test_discover_counts_capacity_per_zone() {
        let topology = discover_cluster_topology(&three_zone_source()).unwrap();
        assert_eq!(topology.zone_count(), 3);
        assert_eq!(topology.node_count(), 9);
        assert_eq!(topology.capacity_in("us-east-1a"), 3);
        assert_eq!(topology.min_zone_capacity(), 3);
        assert_eq!(topology.zones(), vec!["us-east-1a", "us-east-1b", "us-east-1c"]);
    }

    #[test]
    fn test_discover_skips_not_ready_and_cordoned_nodes() {
        let nodes = vec![
            NodeInfo::new("a", Some("zone-a".to_string())),
            NodeInfo::new("b", Some("zone-b".to_string())).with_status(false, false),
            NodeInfo::new("c", Some("zone-c".to_string())).with_status(true, true),
            NodeInfo::new("d", None).with_status(false, true),
        ];
        let topology = discover_cluster_topology(&nodes).unwrap();
        assert_eq!(topology.node_count(), 1);
        assert_eq!(topology.zone_count(), 1);
        assert_eq!(topology.capacity_in("zone-a"), 1);
        assert_eq!(topology.capacity_in("zone-b"), 0);
        assert_eq!(topology.unlabeled_node_count(), 0);
    }

    #[test]
    fn test_discover_counts_unlabeled_schedulable_nodes() {
        let nodes = vec![
            NodeInfo::new("a", Some("zone-a".to_string())),
            NodeInfo::new("b", Some(String::new())),
            NodeInfo::new("c", None),
        ];
        let topology = discover_cluster_topology(&nodes).unwrap();
        assert_eq!(topology.node_count(), 3);
        assert_eq!(topology.unlabeled_node_count(), 2);
        assert_eq!(topology.zone_count(), 1);
    }

    #[test]
    fn test_discover_from_raw_label_maps() {
        let mut labelled = BTreeMap::new();
        labelled.insert(
            TOPOLOGY_ZONE_KEY.to_string(),
            "us-east-1a".to_string(),
        );
        let unlabelled = BTreeMap::new();
        let source = StaticNodeZoneSource::from_label_maps(&[labelled, unlabelled]);
        let topology = discover_cluster_topology(&source).unwrap();
        assert_eq!(topology.capacity_in("us-east-1a"), 1);
        assert_eq!(topology.unlabeled_node_count(), 1);
        assert_eq!(topology.node_count(), 2);
    }

    // ── build_statefulset_patch ───────────────────────────────────────────

    #[test]
    fn test_patch_hard_mode_contains_zone_and_host_keys() {
        let topology = discover_cluster_topology(&three_zone_source()).unwrap();
        let rule_set = build_rule_set(
            &topology,
            BTreeMap::new(),
            Some("stellar"),
            6,
        );
        let patch = build_statefulset_patch(&rule_set, topology.zone_count(), topology.node_count())
            .unwrap();

        let constraints = &patch["spec"]["template"]["spec"]["topologySpreadConstraints"];
        let constraints = constraints.as_array().unwrap();
        assert_eq!(constraints.len(), 2);
        let keys: Vec<&str> = constraints
            .iter()
            .filter_map(|c| c["topologyKey"].as_str())
            .collect();
        assert!(keys.contains(&TOPOLOGY_ZONE_KEY));
        assert!(keys.contains(&TOPOLOGY_HOST_KEY));
        assert!(constraints
            .iter()
            .all(|c| c["whenUnsatisfiable"] == "DoNotSchedule"));
    }

    #[test]
    fn test_patch_soft_mode_single_host_constraint() {
        let topology = discover_cluster_topology(&single_zone_source()).unwrap();
        let rule_set = build_rule_set(&topology, BTreeMap::new(), None, 3);
        let patch = build_statefulset_patch(&rule_set, topology.zone_count(), topology.node_count())
            .unwrap();

        let constraints = patch["spec"]["template"]["spec"]["topologySpreadConstraints"]
            .as_array()
            .unwrap();
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0]["topologyKey"], TOPOLOGY_HOST_KEY);
        assert_eq!(constraints[0]["whenUnsatisfiable"], "ScheduleAnyway");
    }

    #[test]
    fn test_patch_records_mode_and_counts_in_annotations() {
        let topology = discover_cluster_topology(&three_zone_source()).unwrap();
        let rule_set = build_rule_set(&topology, BTreeMap::new(), Some("stellar"), 6);
        let patch = build_statefulset_patch(&rule_set, topology.zone_count(), topology.node_count())
            .unwrap();

        assert_eq!(
            patch["metadata"]["annotations"][TOPOLOGY_MODE_ANNOTATION],
            "HardMultiZone"
        );
        assert_eq!(
            patch["metadata"]["annotations"][TOPOLOGY_ZONES_ANNOTATION],
            "3"
        );
        assert_eq!(
            patch["metadata"]["annotations"][TOPOLOGY_NODES_ANNOTATION],
            "9"
        );
        assert!(!patch["spec"]["template"]["spec"]["affinity"]["podAntiAffinity"].is_null());
    }

    // ── enforce_statefulset_manifest: multi-zone scenario ─────────────────

    #[test]
    fn test_manifest_multi_zone_generates_hard_rules() {
        let mut manifest = statefulset_manifest("validator", "stellar", 6);
        let result =
            enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false).unwrap();

        assert_eq!(result.statefulset_name, "validator");
        assert_eq!(result.namespace, "stellar");
        assert_eq!(result.mode, TopologyMode::HardMultiZone);
        assert_eq!(result.zones.len(), 3);
        assert_eq!(result.replicas, 6);
        assert!(result.patched);
        assert!(!result.dry_run);

        let constraints = spread_constraints(&manifest);
        assert_eq!(constraints.len(), 2);
        assert!(constraints
            .iter()
            .all(|c| c["whenUnsatisfiable"] == "DoNotSchedule"));
        assert_eq!(
            constraints[0]["topologyKey"],
            TOPOLOGY_ZONE_KEY,
            "zone spread must come first"
        );
        assert_eq!(
            constraints[0]["labelSelector"]["matchLabels"]["app.kubernetes.io/name"],
            "stellar-node"
        );

        // Hard (required) host anti-affinity, no soft side.
        let anti = &manifest["spec"]["template"]["spec"]["affinity"]["podAntiAffinity"];
        assert!(anti["requiredDuringSchedulingIgnoredDuringExecution"].is_array());
        assert!(anti.get("preferredDuringSchedulingIgnoredDuringExecution").is_none());
        assert_eq!(
            anti["requiredDuringSchedulingIgnoredDuringExecution"][0]["topologyKey"],
            TOPOLOGY_HOST_KEY
        );

        // Annotations recorded, pre-existing annotations preserved.
        assert_eq!(
            manifest["metadata"]["annotations"][TOPOLOGY_MODE_ANNOTATION],
            "HardMultiZone"
        );
        assert_eq!(
            manifest["metadata"]["annotations"][TOPOLOGY_ZONES_ANNOTATION],
            "3"
        );
        assert_eq!(manifest["metadata"]["annotations"]["stellar.org/keep-me"], "yes");
        // The rest of the manifest is untouched.
        assert_eq!(manifest["kind"], "StatefulSet");
        assert_eq!(manifest["spec"]["replicas"], 6);
    }

    // ── enforce_statefulset_manifest: single-zone dev scenario ────────────

    #[test]
    fn test_manifest_single_zone_falls_back_to_soft_rules() {
        let mut manifest = statefulset_manifest("validator", "stellar", 3);
        let result =
            enforce_statefulset_manifest(&mut manifest, &single_zone_source(), false).unwrap();

        assert_eq!(result.mode, TopologyMode::SoftSingleZone);
        assert_eq!(result.zones, vec!["local"]);

        let constraints = spread_constraints(&manifest);
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0]["topologyKey"], TOPOLOGY_HOST_KEY);
        assert_eq!(constraints[0]["whenUnsatisfiable"], "ScheduleAnyway");

        let anti = &manifest["spec"]["template"]["spec"]["affinity"]["podAntiAffinity"];
        assert!(anti["preferredDuringSchedulingIgnoredDuringExecution"].is_array());
        assert!(anti.get("requiredDuringSchedulingIgnoredDuringExecution").is_none());
        assert_eq!(
            manifest["metadata"]["annotations"][TOPOLOGY_MODE_ANNOTATION],
            "SoftSingleZone"
        );
    }

    #[test]
    fn test_manifest_unlabeled_cluster_falls_back_to_soft_rules() {
        let mut manifest = statefulset_manifest("validator", "stellar", 2);
        let result = enforce_statefulset_manifest(
            &mut manifest,
            &StaticNodeZoneSource::default(),
            false,
        )
        .unwrap();
        assert_eq!(result.mode, TopologyMode::SoftSingleZone);
        assert!(result.zones.is_empty());
        assert_eq!(spread_constraints(&manifest).len(), 1);
    }

    // ── capacity gate ─────────────────────────────────────────────────────

    #[test]
    fn test_manifest_capacity_gate_downgrades_hard_to_soft() {
        // 3 zones but only 1 schedulable node per zone; 5 replicas could never
        // satisfy required host anti-affinity, so soft rules must win.
        let mut manifest = statefulset_manifest("validator", "stellar", 5);
        let source = StaticNodeZoneSource::from_zone_labels([
            "us-east-1a",
            "us-east-1b",
            "us-east-1c",
        ]);
        let result = enforce_statefulset_manifest(&mut manifest, &source, false).unwrap();

        assert_eq!(result.mode, TopologyMode::SoftSingleZone);
        assert_eq!(result.node_count, 3);
        let anti = &manifest["spec"]["template"]["spec"]["affinity"]["podAntiAffinity"];
        assert!(anti.get("preferredDuringSchedulingIgnoredDuringExecution").is_some());
        assert!(anti.get("requiredDuringSchedulingIgnoredDuringExecution").is_none());
    }

    // ── idempotency & dry-run ─────────────────────────────────────────────

    #[test]
    fn test_enforcement_is_idempotent() {
        let mut manifest = statefulset_manifest("validator", "stellar", 6);
        let first =
            enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false).unwrap();
        assert!(first.patched);

        let snapshot = manifest.clone();
        let second =
            enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false).unwrap();
        assert!(!second.patched, "second run must be a no-op");
        assert_eq!(manifest, snapshot);
    }

    #[test]
    fn test_dry_run_does_not_mutate_manifest() {
        let mut manifest = statefulset_manifest("validator", "stellar", 6);
        let snapshot = manifest.clone();
        let result =
            enforce_statefulset_manifest(&mut manifest, &three_zone_source(), true).unwrap();

        assert!(result.dry_run);
        assert!(!result.patched);
        assert_eq!(manifest, snapshot);
        assert!(manifest
            .pointer("/spec/template/spec/topologySpreadConstraints")
            .is_none());
    }

    // ── manifest validation ───────────────────────────────────────────────

    #[test]
    fn test_rejects_non_statefulset_kind() {
        let mut manifest = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "nope" }
        });
        let err = enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false)
            .unwrap_err();
        assert!(matches!(err, TopologyError::InvalidManifest(_)));
    }

    #[test]
    fn test_rejects_non_object_manifest() {
        let mut manifest = json!("not a manifest");
        let err = enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false)
            .unwrap_err();
        assert!(matches!(err, TopologyError::InvalidManifest(_)));
    }

    #[test]
    fn test_selector_falls_back_to_pod_template_labels() {
        let mut manifest = json!({
            "apiVersion": "apps/v1",
            "kind": "StatefulSet",
            "metadata": { "name": "horizon", "namespace": "stellar" },
            "spec": {
                "replicas": 3,
                "template": {
                    "metadata": { "labels": { "app.kubernetes.io/name": "horizon" } },
                    "spec": { "containers": [] }
                }
            }
        });
        enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false).unwrap();
        let constraints = spread_constraints(&manifest);
        assert_eq!(
            constraints[0]["labelSelector"]["matchLabels"]["app.kubernetes.io/name"],
            "horizon"
        );
    }

    #[test]
    fn test_existing_pod_spread_constraints_are_replaced() {
        let mut manifest = statefulset_manifest("validator", "stellar", 6);
        manifest["spec"]["template"]["spec"]["topologySpreadConstraints"] =
            json!([{ "maxSkew": 5, "topologyKey": "kubernetes.io/os" }]);
        enforce_statefulset_manifest(&mut manifest, &three_zone_source(), false).unwrap();
        let constraints = spread_constraints(&manifest);
        assert_eq!(constraints.len(), 2);
        assert_eq!(constraints[0]["maxSkew"], 1);
    }

    // ── scheduling validation: even distribution across zones ─────────────

    /// Greedy scheduler honouring the generated rule set.
    ///
    /// Hard rules (`DoNotSchedule` constraints and required host
    /// anti-affinity) act as admission filters; soft rules only influence the
    /// pick order.  Returns the number of pods placed per zone.
    fn simulate_placement(
        rule_set: &TopologyRuleSet,
        topology: &ClusterTopology,
        replicas: usize,
        hosts_per_zone: usize,
    ) -> Vec<usize> {
        let hard_hosts = rule_set
            .anti_affinity
            .required_during_scheduling_ignored_during_execution
            .is_some();
        let hard_zone_spread = rule_set.spread_constraints.iter().any(|c| {
            c.topology_key == TOPOLOGY_ZONE_KEY
                && c.when_unsatisfiable == WhenUnsatisfiable::DoNotSchedule
        });

        let zones = topology.zones();
        let mut zone_load = vec![0usize; zones.len()];
        let mut host_load = vec![vec![0usize; hosts_per_zone]; zones.len()];

        for _ in 0..replicas {
            let mut candidates: Vec<(usize, usize)> = (0..zones.len())
                .flat_map(|z| (0..hosts_per_zone).map(move |h| (z, h)))
                .collect();
            candidates.sort_by_key(|(z, h)| (zone_load[*z], host_load[*z][*h]));

            let feasible = candidates.into_iter().find(|(z, h)| {
                let host_ok = !hard_hosts || host_load[*z][*h] == 0;
                let zone_ok = !hard_zone_spread || {
                    let mut probe = zone_load.clone();
                    probe[*z] += 1;
                    let max = *probe.iter().max().unwrap_or(&0);
                    let min = *probe.iter().min().unwrap_or(&0);
                    max - min <= 1
                };
                host_ok && zone_ok
            });

            let (z, h) = feasible.expect("hard rules must admit every replica");
            zone_load[z] += 1;
            host_load[z][h] += 1;
        }

        zone_load
    }

    #[test]
    fn test_three_zone_cluster_distributes_validator_pods_evenly() {
        let topology = discover_cluster_topology(&three_zone_source()).unwrap();
        let selector = BTreeMap::from([(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        )]);
        let rule_set = build_rule_set(&topology, selector, Some("stellar"), 6);
        assert_eq!(rule_set.mode, TopologyMode::HardMultiZone);

        let distribution = simulate_placement(&rule_set, &topology, 6, 3);
        assert_eq!(distribution.iter().sum::<usize>(), 6);
        let max = *distribution.iter().max().unwrap();
        let min = *distribution.iter().min().unwrap();
        assert!(
            max - min <= 1,
            "hard zone spread must keep validator pods evenly distributed: {distribution:?}"
        );
        assert!(distribution.iter().all(|load| *load == 2));
    }

    #[test]
    fn test_single_zone_soft_rules_keep_scheduling() {
        let topology = discover_cluster_topology(&single_zone_source()).unwrap();
        let rule_set = build_rule_set(&topology, BTreeMap::new(), None, 3);
        assert_eq!(rule_set.mode, TopologyMode::SoftSingleZone);

        // Two hosts, three replicas: soft rules must not deadlock scheduling,
        // so all replicas are still placed.
        let distribution = simulate_placement(&rule_set, &topology, 3, 2);
        assert_eq!(distribution.iter().sum::<usize>(), 3);
        assert_eq!(distribution, vec![3]);
    }
}

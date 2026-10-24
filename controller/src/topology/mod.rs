// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! Dynamic Node Topology Spread Constraint Enforcer.
//!
//! This module provides the types and logic for inspecting cluster node
//! availability-zone labels and generating Kubernetes `topologySpreadConstraints`
//! and `podAntiAffinity` rules for Stellar-node StatefulSets.
//!
//! See [`enforcer`] for the mutating-controller entry points and [`rules`]
//! for the rule-generation logic.

pub mod enforcer;
pub mod rules;

pub use enforcer::{
    build_statefulset_patch, discover_cluster_topology, enforce_statefulset_manifest,
    EnforcementResult, NodeInfo, NodeZoneSource, StaticNodeZoneSource, TopologyError,
    TOPOLOGY_MODE_ANNOTATION, TOPOLOGY_NODES_ANNOTATION, TOPOLOGY_ZONES_ANNOTATION,
};
pub use rules::{
    build_rule_set, hard_host_anti_affinity, soft_host_anti_affinity, ClusterTopology,
    LabelSelector, PodAffinityTerm, PodAntiAffinity, TopologyMode, TopologyRuleSet,
    TopologySpreadConstraint, WhenUnsatisfiable, WeightedPodAffinityTerm,
    MIN_ZONES_FOR_HARD_SPREAD, TOPOLOGY_HOST_KEY, TOPOLOGY_ZONE_KEY,
};

//! PDB auto-tuning calculator.
//!
//! Pure, side-effect free logic that decides the `maxUnavailable` value for a
//! validator deployment group's `PodDisruptionBudget` based on the live sync
//! health of the Stellar nodes that back it.
//!
//! The reconciler (`controller/src/pdb/reconciler.rs`) is responsible for
//! watching `StellarNode` custom resources, querying their sync health and
//! applying the value produced here. Keeping the decision logic isolated makes
//! it straightforward to unit test the safety-critical quorum rules.

use std::collections::HashSet;

/// Sync health of a single Stellar node, as reported by the node's SCP state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// The node is actively syncing or catching up on ledgers.
    Syncing,
    /// The node is fully synced and stable.
    Synced,
    /// The node's health could not be determined (missing metrics, errors).
    Unknown,
}

/// A single Stellar node that participates in a validator deployment group.
#[derive(Debug, Clone)]
pub struct NodeHealth {
    /// Stable identifier of the node (e.g. `StellarNode` name).
    pub name: String,
    /// Current sync state of the node.
    pub sync_state: SyncState,
}

/// The active cluster topology used to enforce the minimum quorum.
#[derive(Debug, Clone)]
pub struct ClusterTopology {
    /// Total number of validator nodes that make up the quorum.
    pub total_validators: usize,
    /// Minimum number of validators that must remain available to preserve
    /// quorum. Must be `<= total_validators`.
    pub min_quorum: usize,
}

/// The outcome of a PDB calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdbDecision {
    /// The `maxUnavailable` value to write into the `PodDisruptionBudget`.
    pub max_unavailable: u32,
    /// Human readable explanation, useful for events and logs.
    pub reason: String,
}

/// Errors that prevent a safe PDB decision from being produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalculatorError {
    /// The topology is internally inconsistent (e.g. `min_quorum` exceeds the
    /// number of validators).
    InvalidTopology(String),
    /// No nodes were provided for the deployment group.
    EmptyGroup,
}

impl std::fmt::Display for CalculatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalculatorError::InvalidTopology(msg) => {
                write!(f, "invalid cluster topology: {msg}")
            }
            CalculatorError::EmptyGroup => write!(f, "validator deployment group is empty"),
        }
    }
}

impl std::error::Error for CalculatorError {}

/// Computes the `maxUnavailable` value for a validator deployment group.
///
/// Rules (in priority order):
/// 1. If any node in the group is `Syncing` or `Unknown`, the group is not
///    stable and `maxUnavailable` is forced to `0` so that `kubectl drain` is
///    blocked while a validator is catching up.
/// 2. If every node is `Synced`, `maxUnavailable` is set to `1`, but only when
///    doing so cannot violate the minimum quorum required by the active
///    topology. Otherwise it falls back to `0`.
///
/// The function never returns a value that would allow the number of
/// simultaneously evicted validators to drop the cluster below `min_quorum`.
pub fn calculate_max_unavailable(
    nodes: &[NodeHealth],
    topology: &ClusterTopology,
) -> Result<PdbDecision, CalculatorError> {
    if nodes.is_empty() {
        return Err(CalculatorError::EmptyGroup);
    }

    if topology.total_validators == 0 || topology.min_quorum > topology.total_validators {
        return Err(CalculatorError::InvalidTopology(format!(
            "min_quorum ({}) must be between 0 and total_validators ({})",
            topology.min_quorum, topology.total_validators
        )));
    }

    // Rule 1: any node that is not fully synced pins the group to 0.
    let unstable: Vec<&str> = nodes
        .iter()
        .filter(|n| n.sync_state != SyncState::Synced)
        .map(|n| n.name.as_str())
        .collect();

    if !unstable.is_empty() {
        return Ok(PdbDecision {
            max_unavailable: 0,
            reason: format!(
                "holding maxUnavailable=0: {} node(s) not fully synced ({})",
                unstable.len(),
                unstable.join(", ")
            ),
        });
    }

    // Rule 2: all nodes are synced. Only allow a single eviction when the
    // active topology can tolerate it without breaking quorum.
    let headroom = topology.total_validators.saturating_sub(topology.min_quorum);

    if headroom >= 1 {
        Ok(PdbDecision {
            max_unavailable: 1,
            reason: format!(
                "all {} node(s) synced; topology tolerates 1 eviction (min_quorum={})",
                nodes.len(),
                topology.min_quorum
            ),
        })
    } else {
        Ok(PdbDecision {
            max_unavailable: 0,
            reason: format!(
                "all {} node(s) synced but topology has no quorum headroom (min_quorum={}); holding maxUnavailable=0",
                nodes.len(),
                topology.min_quorum
            ),
        })
    }
}

/// Convenience helper that derives the topology from a set of validator node
/// names and a minimum quorum size.
///
/// This is used by the reconciler when it needs to build a [`ClusterTopology`]
/// from the `StellarNode` resources it is watching.
pub fn topology_from_validators<'a, I>(validators: I, min_quorum: usize) -> ClusterTopology
where
    I: IntoIterator<Item = &'a str>,
{
    let unique: HashSet<&str> = validators.into_iter().collect();
    ClusterTopology {
        total_validators: unique.len(),
        min_quorum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, sync_state: SyncState) -> NodeHealth {
        NodeHealth {
            name: name.to_string(),
            sync_state,
        }
    }

    fn topology(total: usize, min_quorum: usize) -> ClusterTopology {
        ClusterTopology {
            total_validators: total,
            min_quorum,
        }
    }

    #[test]
    fn syncing_node_forces_zero() {
        let nodes = vec![
            node("validator-0", SyncState::Synced),
            node("validator-1", SyncState::Syncing),
        ];
        let decision = calculate_max_unavailable(&nodes, &topology(3, 2)).unwrap();
        assert_eq!(decision.max_unavailable, 0);
    }

    #[test]
    fn unknown_node_forces_zero() {
        let nodes = vec![node("validator-0", SyncState::Unknown)];
        let decision = calculate_max_unavailable(&nodes, &topology(3, 2)).unwrap();
        assert_eq!(decision.max_unavailable, 0);
    }

    #[test]
    fn fully_synced_with_headroom_allows_one() {
        let nodes = vec![
            node("validator-0", SyncState::Synced),
            node("validator-1", SyncState::Synced),
            node("validator-2", SyncState::Synced),
        ];
        let decision = calculate_max_unavailable(&nodes, &topology(3, 2)).unwrap();
        assert_eq!(decision.max_unavailable, 1);
    }

    #[test]
    fn fully_synced_without_headroom_holds_zero() {
        let nodes = vec![
            node("validator-0", SyncState::Synced),
            node("validator-1", SyncState::Synced),
        ];
        let decision = calculate_max_unavailable(&nodes, &topology(2, 2)).unwrap();
        assert_eq!(decision.max_unavailable, 0);
    }

    #[test]
    fn empty_group_is_rejected() {
        let err = calculate_max_unavailable(&[], &topology(3, 2)).unwrap_err();
        assert_eq!(err, CalculatorError::EmptyGroup);
    }

    #[test]
    fn invalid_topology_is_rejected() {
        let nodes = vec![node("validator-0", SyncState::Synced)];
        let err = calculate_max_unavailable(&nodes, &topology(2, 3)).unwrap_err();
        assert!(matches!(err, CalculatorError::InvalidTopology(_)));
    }

    #[test]
    fn topology_from_validators_deduplicates() {
        let topo = topology_from_validators(["a", "b", "a"], 2);
        assert_eq!(topo.total_validators, 2);
        assert_eq!(topo.min_quorum, 2);
    }
}

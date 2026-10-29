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
//! Readiness-based disruption guard for the legacy PDB tuner.
//!
//! This is a conservative availability heuristic, not an SCP quorum analysis.

/// Block voluntary disruption during catchup or when fewer than two Pods are
/// ready. Otherwise allow at most one ready Pod to be disrupted.
pub fn max_unavailable(any_syncing: bool, ready_count: usize) -> i32 {
    i32::from(!any_syncing && ready_count >= 2)
}

#[cfg(test)]
mod tests {
    use super::max_unavailable;

    #[test]
    fn catchup_blocks_disruption_even_with_many_ready_pods() {
        assert_eq!(max_unavailable(true, 5), 0);
    }

    #[test]
    fn a_single_ready_replica_is_protected() {
        assert_eq!(max_unavailable(false, 0), 0);
        assert_eq!(max_unavailable(false, 1), 0);
    }

    #[test]
    fn synchronized_replicas_allow_only_one_disruption() {
        assert_eq!(max_unavailable(false, 2), 1);
        assert_eq!(max_unavailable(false, 5), 1);
    }
}

/// Legacy quorum-count helper retained for existing callers.
pub fn f(catching_up: bool, synchronized: usize, quorum: usize) -> usize {
    if catching_up { 0 } else { synchronized.saturating_sub(quorum) }
}

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

#[path = "../src/leader.rs"]
mod leader;

use leader::{StatefulSetLeaderElector, ValidatorRole};

#[test]
fn test_statefulset_leader_election_failover_and_split_brain_fencing() {
    assert!(leader::DEFAULT_RENEW_DEADLINE_MS > leader::DEFAULT_RETRY_PERIOD_MS);
    let mut elector = StatefulSetLeaderElector::new("stellar-validator-lease", "stellar-prod");

    // 1. stellar-core-0 acquires initial Lease (fencing token = 1)
    let pod0_initial = elector.reconcile_pod_role("stellar-core-0", 1_000);
    assert_eq!(pod0_initial.role, ValidatorRole::ActiveSigner);
    assert!(pod0_initial.node_is_validator_env);
    assert_eq!(pod0_initial.fencing_token, 1);

    // 2. Standby pod stellar-core-1 checks in while lease is valid -> PassiveObserver
    let pod1_standby = elector.reconcile_pod_role("stellar-core-1", 4_000);
    assert_eq!(pod1_standby.role, ValidatorRole::PassiveObserver);
    assert!(!pod1_standby.node_is_validator_env);

    // 3. Simulate network partition / crash of stellar-core-0 (> 9s lease duration, < 10s SLA)
    let pod1_failover = elector.reconcile_pod_role("stellar-core-1", 10_500);
    assert_eq!(pod1_failover.role, ValidatorRole::ActiveSigner);
    assert!(pod1_failover.node_is_validator_env);
    assert_eq!(pod1_failover.fencing_token, 2);

    // 4. Split-brain protection: partitioned stellar-core-0 with stale token 1 is rejected
    assert!(!elector.verify_signer_fencing_token("stellar-core-0", 1, 11_000));
    assert!(elector.verify_signer_fencing_token("stellar-core-1", 2, 11_000));

    // 5. When stellar-core-0 reconnects, it is demoted to PassiveObserver
    let pod0_rejoined = elector.reconcile_pod_role("stellar-core-0", 11_500);
    assert_eq!(pod0_rejoined.role, ValidatorRole::PassiveObserver);
    assert!(!pod0_rejoined.node_is_validator_env);
}

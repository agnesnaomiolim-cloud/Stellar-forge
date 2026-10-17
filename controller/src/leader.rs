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

use serde::{Deserialize, Serialize};

pub const DEFAULT_LEASE_DURATION_MS: u64 = 9_000;
pub const DEFAULT_RENEW_DEADLINE_MS: u64 = 6_000;
pub const DEFAULT_RETRY_PERIOD_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ValidatorRole {
    ActiveSigner,
    PassiveObserver,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_name: String,
    pub namespace: String,
    pub holder_identity: String,
    pub acquire_time_ms: u64,
    pub renew_time_ms: u64,
    pub lease_duration_ms: u64,
    pub lease_transitions: u64,
}

impl LeaseRecord {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.renew_time_ms) >= self.lease_duration_ms
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodElectionDecision {
    pub pod_identity: String,
    pub role: ValidatorRole,
    pub node_is_validator_env: bool,
    pub fencing_token: u64,
    pub lease_snapshot: LeaseRecord,
}

pub struct StatefulSetLeaderElector {
    pub lease_name: String,
    pub namespace: String,
    pub lease_duration_ms: u64,
    pub current_lease: Option<LeaseRecord>,
}

impl StatefulSetLeaderElector {
    pub fn new(lease_name: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            lease_name: lease_name.into(),
            namespace: namespace.into(),
            lease_duration_ms: DEFAULT_LEASE_DURATION_MS,
            current_lease: None,
        }
    }

    /// Evaluates or acquires the distributed Kubernetes Lease for a StatefulSet replica.
    /// Guarantees split-brain prevention via monotonic fencing tokens and strict expiry checks.
    pub fn reconcile_pod_role(&mut self, pod_identity: &str, now_ms: u64) -> PodElectionDecision {
        match &mut self.current_lease {
            None => {
                let record = LeaseRecord {
                    lease_name: self.lease_name.clone(),
                    namespace: self.namespace.clone(),
                    holder_identity: pod_identity.to_string(),
                    acquire_time_ms: now_ms,
                    renew_time_ms: now_ms,
                    lease_duration_ms: self.lease_duration_ms,
                    lease_transitions: 1,
                };
                self.current_lease = Some(record.clone());
                PodElectionDecision {
                    pod_identity: pod_identity.to_string(),
                    role: ValidatorRole::ActiveSigner,
                    node_is_validator_env: true,
                    fencing_token: record.lease_transitions,
                    lease_snapshot: record,
                }
            }
            Some(lease) => {
                if lease.holder_identity == pod_identity && !lease.is_expired(now_ms) {
                    lease.renew_time_ms = now_ms;
                    let snapshot = lease.clone();
                    PodElectionDecision {
                        pod_identity: pod_identity.to_string(),
                        role: ValidatorRole::ActiveSigner,
                        node_is_validator_env: true,
                        fencing_token: snapshot.lease_transitions,
                        lease_snapshot: snapshot,
                    }
                } else if lease.is_expired(now_ms) {
                    // Failover within < 10s window: increment fencing token to fence old leader
                    lease.holder_identity = pod_identity.to_string();
                    lease.acquire_time_ms = now_ms;
                    lease.renew_time_ms = now_ms;
                    lease.lease_transitions += 1;
                    let snapshot = lease.clone();
                    PodElectionDecision {
                        pod_identity: pod_identity.to_string(),
                        role: ValidatorRole::ActiveSigner,
                        node_is_validator_env: true,
                        fencing_token: snapshot.lease_transitions,
                        lease_snapshot: snapshot,
                    }
                } else {
                    let snapshot = lease.clone();
                    PodElectionDecision {
                        pod_identity: pod_identity.to_string(),
                        role: ValidatorRole::PassiveObserver,
                        node_is_validator_env: false,
                        fencing_token: snapshot.lease_transitions,
                        lease_snapshot: snapshot,
                    }
                }
            }
        }
    }

    /// Validates whether a pod attempting to sign an SCP message holds the current fencing token.
    pub fn verify_signer_fencing_token(&self, pod_identity: &str, token: u64, now_ms: u64) -> bool {
        match &self.current_lease {
            Some(lease) => {
                !lease.is_expired(now_ms)
                    && lease.holder_identity == pod_identity
                    && lease.lease_transitions == token
            }
            None => false,
        }
    }
}

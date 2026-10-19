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
//! Namespace-scoped readiness tuner for the legacy `app=stellar` PDB.
//!
//! This tuner is separate from the primary per-StellarNode resource builder.
//! It uses Pod readiness and a sync annotation, not full Stellar quorum state.

use super::health;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::time::Duration;
use tracing::{error, info};

pub const PDB_NAME: &str = "stellar-pdb";
const STELLAR_LABEL: &str = "app=stellar";
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 30;

pub struct PdbReconciler {
    client: Client,
    namespace: String,
}

impl PdbReconciler {
    pub fn new(client: Client) -> Self {
        Self::with_namespace(
            client,
            env::var("NAMESPACE").unwrap_or_else(|_| "default".to_string()),
        )
    }

    pub fn with_namespace(client: Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    pub async fn run(&self) {
        let interval = env::var("RECONCILE_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .unwrap_or(DEFAULT_RECONCILE_INTERVAL_SECS);
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        loop {
            ticker.tick().await;
            if let Err(error) = self.reconcile().await {
                error!(?error, namespace = %self.namespace, "PDB reconciliation failed");
            }
        }
    }

    pub async fn reconcile(&self) -> Result<(), kube::Error> {
        reconcile(&self.client, &self.namespace).await
    }
}

fn desired_max_unavailable(pods: &[Pod]) -> i32 {
    let any_syncing = pods.iter().any(|pod| {
        pod.metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get("stellar-sync-status"))
            .is_some_and(|status| status == "syncing")
    });
    let ready_count = pods
        .iter()
        .filter(|pod| {
            pod.metadata.deletion_timestamp.is_none()
                && pod.status.as_ref().is_some_and(|status| {
                    status.phase.as_deref() == Some("Running")
                        && status.conditions.as_ref().is_some_and(|conditions| {
                            conditions.iter().any(|condition| {
                                condition.type_ == "Ready" && condition.status == "True"
                            })
                        })
                })
        })
        .count();
    health::max_unavailable(any_syncing, ready_count)
}

/// Inspect only Pods in the PDB namespace, then create or patch its budget.
/// Missing readiness is fail-closed; syncing Pods always block disruption.
pub async fn reconcile(client: &Client, namespace: &str) -> Result<(), kube::Error> {
    let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pods_api
        .list(&ListParams::default().labels(STELLAR_LABEL))
        .await?;
    let max_unavailable = desired_max_unavailable(&pods.items);
    let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), namespace);
    let selector = LabelSelector {
        match_labels: Some(BTreeMap::from([("app".to_string(), "stellar".to_string())])),
        ..Default::default()
    };
    match pdb_api.get(PDB_NAME).await {
        Ok(_) => {
            // Merge patch preserves unrelated metadata; no invalid partial SSA body.
            pdb_api.patch(PDB_NAME, &PatchParams::default(), &Patch::Merge(json!({
                "spec": {"maxUnavailable": max_unavailable, "minAvailable": null, "selector": selector}
            }))).await?;
        }
        Err(kube::Error::Api(error)) if error.code == 404 => {
            let pdb = PodDisruptionBudget {
                metadata: ObjectMeta {
                    name: Some(PDB_NAME.to_string()),
                    namespace: Some(namespace.to_string()),
                    ..Default::default()
                },
                spec: Some(PodDisruptionBudgetSpec {
                    max_unavailable: Some(IntOrString::Int(max_unavailable)),
                    selector: Some(selector),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pdb_api.create(&PostParams::default(), &pdb).await?;
        }
        Err(error) => return Err(error),
    }
    info!(namespace, max_unavailable, "Reconciled legacy Stellar PDB");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{PodCondition, PodStatus};

    fn ready_pod() -> Pod {
        Pod {
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn missing_readiness_does_not_count_as_healthy() {
        assert_eq!(desired_max_unavailable(&[ready_pod(), Pod::default()]), 0);
    }

    #[test]
    fn syncing_replica_prevents_eviction_until_annotation_clears() {
        let mut syncing = ready_pod();
        syncing.metadata.annotations = Some(BTreeMap::from([(
            "stellar-sync-status".to_string(),
            "syncing".to_string(),
        )]));
        assert_eq!(desired_max_unavailable(&[ready_pod(), syncing.clone()]), 0);
        syncing.metadata.annotations = None;
        assert_eq!(desired_max_unavailable(&[ready_pod(), syncing]), 1);
    }
}

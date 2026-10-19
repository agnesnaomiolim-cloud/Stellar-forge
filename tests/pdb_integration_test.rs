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
//! Live-cluster PDB eviction contract. Run explicitly with `--ignored`.

mod common;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, EvictParams, ListParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use stellar_k8s::controller::pdb::reconciler::{reconcile, PDB_NAME};

async fn wait_for_budget(api: &Api<PodDisruptionBudget>, allowed: i32) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let pdb = api.get(PDB_NAME).await.expect("PDB must exist");
            if pdb.status.as_ref().is_some_and(|status| {
                status.observed_generation == pdb.metadata.generation
                    && status.disruptions_allowed == allowed
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("PDB controller did not observe the expected disruption budget");
}

#[tokio::test]
#[ignore = "requires a live Kubernetes cluster and kubectl for namespace cleanup"]
async fn pdb_blocks_eviction_during_sync() {
    let client = Client::try_default()
        .await
        .expect("failed to create kube client");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let namespace = format!("pdb-integration-{unique}");
    let namespaces: Api<Namespace> = Api::all(client.clone());
    namespaces
        .create(
            &PostParams::default(),
            &Namespace {
                metadata: ObjectMeta {
                    name: Some(namespace.clone()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .expect("create isolated test namespace");
    let _namespace_guard = common::NamespaceGuard {
        name: namespace.clone(),
    };

    // A scalable owner is required by Kubernetes' percentage/maxUnavailable PDB
    // controller. Bare Pods would not exercise a valid eviction policy.
    let deployments: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let deployment: Deployment = serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "validators"},
        "spec": {"replicas": 2, "selector": {"matchLabels": {"app": "stellar"}},
            "template": {"metadata": {"labels": {"app": "stellar"}}, "spec": {
                "securityContext": {"runAsNonRoot": true, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}},
                "containers": [{"name": "validator", "image": "registry.k8s.io/pause:3.10",
                    "securityContext": {"allowPrivilegeEscalation": false, "capabilities": {"drop": ["ALL"]}}}]
            }}
        }
    })).unwrap();
    deployments
        .create(&PostParams::default(), &deployment)
        .await
        .expect("create replica owner");
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let ready_pods = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let list = pods
                .list(&ListParams::default().labels("app=stellar"))
                .await
                .expect("list replica Pods");
            if list.items.len() == 2
                && list.items.iter().all(|pod| {
                    pod.status
                        .as_ref()
                        .and_then(|status| status.conditions.as_ref())
                        .is_some_and(|conditions| {
                            conditions.iter().any(|condition| {
                                condition.type_ == "Ready" && condition.status == "True"
                            })
                        })
                })
            {
                break list.items;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("replicas did not become Ready");
    let pod_name = ready_pods[0].metadata.name.as_deref().unwrap();
    pods.patch(
        pod_name,
        &PatchParams::default(),
        &Patch::Merge(json!({
            "metadata": {"annotations": {"stellar-sync-status": "syncing"}}
        })),
    )
    .await
    .expect("mark catchup in progress");

    reconcile(&client, &namespace)
        .await
        .expect("reconcile syncing budget");
    let pdbs: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);
    let pdb = pdbs.get(PDB_NAME).await.unwrap();
    assert_eq!(pdb.spec.unwrap().max_unavailable, Some(IntOrString::Int(0)));
    wait_for_budget(&pdbs, 0).await;
    let dry_run = EvictParams {
        post_options: PostParams {
            dry_run: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let denied = pods
        .evict(pod_name, &dry_run)
        .await
        .expect_err("catchup must block eviction");
    assert!(
        matches!(denied, kube::Error::Api(error) if error.code == 429),
        "eviction must fail specifically because of its budget"
    );

    pods.patch(
        pod_name,
        &PatchParams::default(),
        &Patch::Merge(json!({
            "metadata": {"annotations": {"stellar-sync-status": "synced"}}
        })),
    )
    .await
    .expect("mark catchup complete");
    reconcile(&client, &namespace)
        .await
        .expect("reconcile healthy budget");
    let pdb = pdbs.get(PDB_NAME).await.unwrap();
    assert_eq!(pdb.spec.unwrap().max_unavailable, Some(IntOrString::Int(1)));
    wait_for_budget(&pdbs, 1).await;
    pods.evict(pod_name, &dry_run)
        .await
        .expect("a healthy replica may be evicted");
}

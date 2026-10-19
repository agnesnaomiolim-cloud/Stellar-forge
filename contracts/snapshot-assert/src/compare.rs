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
use soroban_sdk::{contracttype, Env, String, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotComparison {
    pub ledger_index: u32,
    pub reference_cluster: String,
    pub reference_root: String,
    pub conflicting_clusters: Vec<String>,
    pub has_divergence: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub ledger_index: u32,
    pub cluster_id: String,
    pub expected_root: String,
    pub actual_root: String,
}

pub fn compare_roots(
    env: &Env,
    ledger_index: u32,
    roots: Vec<(String, String)>,
) -> SnapshotComparison {
    if roots.is_empty() {
        return SnapshotComparison {
            ledger_index,
            reference_cluster: String::from_str(env, ""),
            reference_root: String::from_str(env, ""),
            conflicting_clusters: Vec::new(env),
            has_divergence: false,
        };
    }

    let reference_cluster = roots.get(0).unwrap().0.clone();
    let reference_root = roots.get(0).unwrap().1.clone();
    let mut conflicting = Vec::new(env);

    for idx in 1..roots.len() {
        let (_, candidate_root) = roots.get(idx).unwrap();
        if candidate_root != reference_root {
            let cluster = roots.get(idx).unwrap().0.clone();
            conflicting.push_back(cluster);
        }
    }

    let has_divergence = !conflicting.is_empty();
    SnapshotComparison {
        ledger_index,
        reference_cluster,
        reference_root,
        conflicting_clusters: conflicting,
        has_divergence,
    }
}

pub fn assert_snapshot(
    env: &Env,
    ledger_index: u32,
    roots: Vec<(String, String)>,
) -> SnapshotComparison {
    let comparison = compare_roots(env, ledger_index, roots);
    if comparison.has_divergence {
        env.events().publish(
            ("StateDivergenceDetected",),
            (
                ledger_index,
                comparison.reference_cluster.clone(),
                comparison.reference_root.clone(),
                comparison.conflicting_clusters.clone(),
            ),
        );
    }
    comparison
}

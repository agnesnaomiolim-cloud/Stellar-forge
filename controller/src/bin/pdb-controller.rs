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
// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! PDB auto-tuning controller binary.
//!
//! Env: `PDB_RECONCILE_INTERVAL_SECS` (default 30), `PDB_STABILITY_WINDOW_SECS`
//! (default 60), `RUST_LOG`.

use std::time::Duration;

use controller::pdb::kube_backend::{watch_stellar_nodes, KubeCluster};
use controller::pdb::{PdbController, PdbPolicy};

fn env_secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cluster = KubeCluster::try_default().await?;
    let client = kube::Client::try_default().await?;
    let policy = PdbPolicy {
        stability_window: env_secs("PDB_STABILITY_WINDOW_SECS", 60),
    };
    let controller = PdbController::new(cluster, policy);
    tokio::spawn(watch_stellar_nodes(client, controller.trigger()));

    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(true);
    });
    controller
        .run(env_secs("PDB_RECONCILE_INTERVAL_SECS", 30), rx)
        .await;
    Ok(())
}
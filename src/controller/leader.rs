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
//! Leader election helpers backed by Kubernetes Leases.

use chrono::Utc;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, PostParams};
use kube::{Client, Config};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::controller::metrics::set_leader_status;

const DEFAULT_LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;
const RENEW_INTERVAL: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const API_TIMEOUT: Duration = Duration::from_secs(3);

static IS_LEADER: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
enum LeaseError {
    #[error("Kubernetes Lease API request timed out")]
    Timeout,
    #[error(transparent)]
    Kubernetes(#[from] kube::Error),
}

/// Returns `true` if this process currently holds the operator leader lease.
pub fn is_leader() -> bool {
    IS_LEADER.load(Ordering::SeqCst)
}

/// Create a client for the shared coordination cluster when configured.
///
/// Every operator in a multi-cluster deployment must point this at the same
/// Kubernetes API. Without the kubeconfig, election falls back to the local
/// cluster and therefore only coordinates replicas in that cluster.
pub async fn coordination_client(local_client: Client) -> crate::Result<Client> {
    let path = match std::env::var("LEADER_ELECTION_KUBECONFIG") {
        Ok(path) => path,
        Err(std::env::VarError::NotPresent)
            if std::env::var("REQUIRE_SHARED_LEADER_ELECTION")
                .is_ok_and(|value| value.eq_ignore_ascii_case("true")) =>
        {
            return Err(crate::Error::ConfigError(
                "REQUIRE_SHARED_LEADER_ELECTION is true but LEADER_ELECTION_KUBECONFIG is not set"
                    .to_string(),
            ));
        }
        Err(std::env::VarError::NotPresent) => return Ok(local_client),
        Err(error) => {
            return Err(crate::Error::ConfigError(format!(
                "invalid LEADER_ELECTION_KUBECONFIG: {error}"
            )))
        }
    };

    let kubeconfig = kube::config::Kubeconfig::read_from(&path).map_err(|error| {
        crate::Error::ConfigError(format!(
            "failed to read coordination-cluster kubeconfig {path}: {error}"
        ))
    })?;
    let config = Config::from_custom_kubeconfig(
        kubeconfig,
        &kube::config::KubeConfigOptions::default(),
    )
    .await
    .map_err(|error| {
        crate::Error::ConfigError(format!(
            "failed to load coordination-cluster kubeconfig {path}: {error}"
        ))
    })?;
    Client::try_from(config).map_err(crate::Error::KubeError)
}

/// Guard proving a maintenance operation owns its Kubernetes Lease.
pub struct LeaseGuard {
    lease_name: String,
    held: Arc<AtomicBool>,
    renewal_task: JoinHandle<()>,
}

impl LeaseGuard {
    /// Acquire and continuously renew a short-lived Kubernetes Lease.
    pub async fn acquire(scope: impl Into<String>) -> crate::Result<Self> {
        let scope = scope.into();
        let lease_name = format!("stellar-k8s-{scope}");
        let namespace = default_lease_namespace();
        let identity = pod_identity();
        let local_client = Client::try_default().await?;
        let client = coordination_client(local_client).await?;
        let leases: Api<Lease> = Api::namespaced(client.clone(), &namespace);

        match try_acquire_or_renew(&leases, &lease_name, &namespace, &identity)
            .await
            .map_err(|error| crate::Error::ConfigError(error.to_string()))?
        {
            true => {
                let held = Arc::new(AtomicBool::new(true));
                let renewal_task = spawn_guard_renewal(
                    client,
                    lease_name.clone(),
                    namespace,
                    identity,
                    Arc::clone(&held),
                );
                Ok(Self {
                    lease_name,
                    held,
                    renewal_task,
                })
            }
            false => Err(crate::Error::ConfigError(format!(
                "lease {lease_name} is held by another operator"
            ))),
        }
    }

    /// Name of the Kubernetes Lease held by this guard.
    pub fn lease_name(&self) -> &str {
        &self.lease_name
    }

    /// Fail closed after a renewal error or loss of the lease.
    pub fn ensure_held(&self) -> crate::Result<()> {
        if self.held.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(crate::Error::ConfigError(format!(
                "lease {} was lost during the protected operation",
                self.lease_name
            )))
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.held.store(false, Ordering::SeqCst);
        self.renewal_task.abort();
    }
}

fn spawn_guard_renewal(
    client: Client,
    lease_name: String,
    namespace: String,
    identity: String,
    held: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let leases: Api<Lease> = Api::namespaced(client, &namespace);
        loop {
            tokio::time::sleep(RENEW_INTERVAL).await;
            match try_acquire_or_renew(&leases, &lease_name, &namespace, &identity).await {
                Ok(true) => {}
                Ok(false) => {
                    warn!(lease = %lease_name, "Maintenance lease was taken by another holder");
                    held.store(false, Ordering::SeqCst);
                    return;
                }
                Err(error) => {
                    warn!(lease = %lease_name, %error, "Maintenance lease renewal failed");
                    held.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    })
}

/// Handle to the background leader-election task.
pub struct LeaderElectionHandle {
    task: JoinHandle<()>,
    leadership_changed: watch::Receiver<bool>,
    is_leader: Arc<AtomicBool>,
    readiness: Arc<AtomicBool>,
    client: Client,
    lease_name: String,
    namespace: String,
    identity: String,
}

impl LeaderElectionHandle {
    /// Starts a bounded, background lease acquisition and renewal loop.
    pub fn start(
        client: Client,
        lease_name: Option<String>,
        lease_namespace: Option<String>,
        identity: Option<String>,
    ) -> crate::Result<Self> {
        let name = lease_name
            .or_else(|| std::env::var("OPERATOR_LEASE_NAME").ok())
            .unwrap_or_else(|| DEFAULT_LEASE_NAME.to_string());
        let namespace = lease_namespace.unwrap_or_else(default_lease_namespace);
        let holder = identity.unwrap_or_else(pod_identity);
        let (tx, rx) = watch::channel(false);
        let is_leader = Arc::new(AtomicBool::new(false));
        let readiness = Arc::new(AtomicBool::new(true));
        IS_LEADER.store(false, Ordering::SeqCst);

        let task = tokio::spawn(run_leader_election(
            client.clone(),
            name.clone(),
            namespace.clone(),
            holder.clone(),
            tx,
            Arc::clone(&is_leader),
            Arc::clone(&readiness),
        ));

        Ok(Self {
            task,
            leadership_changed: rx,
            is_leader,
            readiness,
            client,
            lease_name: name,
            namespace,
            identity: holder,
        })
    }

    /// Returns `true` if this process currently holds the lease.
    pub fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::SeqCst)
    }

    /// Shared status flag for controller workers that must fail closed.
    pub fn shared_is_leader(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.is_leader)
    }

    /// Shared readiness signal; an unready leader relinquishes the Lease.
    pub fn shared_readiness(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.readiness)
    }

    /// Waits until this process becomes the leader.
    pub async fn wait_until_leader(&self) {
        let mut rx = self.leadership_changed.clone();
        let _ = rx.wait_for(|value| *value).await;
    }

    /// Waits until this process loses leadership.
    pub async fn wait_until_lost(&self) {
        let mut rx = self.leadership_changed.clone();
        while *rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// Stops renewal and releases the Lease if this process still owns it.
    pub async fn shutdown(self) {
        self.task.abort();
        self.is_leader.store(false, Ordering::SeqCst);
        IS_LEADER.store(false, Ordering::SeqCst);
        set_leader_status(false);

        let leases: Api<Lease> = Api::namespaced(self.client.clone(), &self.namespace);
        match release_lease(&leases, &self.lease_name, &self.identity).await {
            Ok(()) => {}
            Err(error) => {
                warn!(lease = %self.lease_name, %error, "Could not release leader Lease during shutdown")
            }
        }
    }
}

impl Drop for LeaderElectionHandle {
    fn drop(&mut self) {
        self.task.abort();
        self.is_leader.store(false, Ordering::SeqCst);
        IS_LEADER.store(false, Ordering::SeqCst);
    }
}

async fn run_leader_election(
    client: Client,
    lease_name: String,
    namespace: String,
    identity: String,
    tx: watch::Sender<bool>,
    is_leader: Arc<AtomicBool>,
    readiness: Arc<AtomicBool>,
) {
    let leases: Api<Lease> = Api::namespaced(client, &namespace);

    loop {
        if !readiness.load(Ordering::SeqCst) {
            if is_leader.load(Ordering::SeqCst) {
                update_leader_state(&is_leader, &tx, false);
                if let Err(error) = release_lease(&leases, &lease_name, &identity).await {
                    warn!(lease = %lease_name, %error, "Could not release unready leader Lease");
                }
            }
            tokio::time::sleep(RETRY_INTERVAL).await;
            continue;
        }

        match try_acquire_or_renew(&leases, &lease_name, &namespace, &identity).await {
            Ok(true) => {
                if !is_leader.load(Ordering::Relaxed) {
                    info!(lease = %lease_name, holder = %identity, "Acquired leadership");
                }
                update_leader_state(&is_leader, &tx, true);
                tokio::time::sleep(RENEW_INTERVAL).await;
            }
            Ok(false) => {
                if is_leader.load(Ordering::Relaxed) {
                    warn!(lease = %lease_name, "Lost leadership");
                }
                update_leader_state(&is_leader, &tx, false);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(error) => {
                warn!(lease = %lease_name, %error, "Leader election request failed; stepping down");
                update_leader_state(&is_leader, &tx, false);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
        }
    }
}

async fn release_lease(
    leases: &Api<Lease>,
    lease_name: &str,
    identity: &str,
) -> Result<(), LeaseError> {
    let mut lease = match bounded_api(leases.get(lease_name)).await {
        Ok(lease) => lease,
        Err(LeaseError::Kubernetes(kube::Error::Api(error))) if error.code == 404 => {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    let spec = lease.spec.get_or_insert_with(LeaseSpec::default);
    if spec.holder_identity.as_deref() != Some(identity) {
        return Ok(());
    }
    spec.holder_identity = None;
    spec.renew_time = Some(MicroTime(Utc::now()));
    bounded_api(leases.replace(lease_name, &PostParams::default(), &lease))
        .await
        .map(|_| ())
}

fn update_leader_state(
    is_leader: &AtomicBool,
    tx: &watch::Sender<bool>,
    value: bool,
) {
    is_leader.store(value, Ordering::SeqCst);
    IS_LEADER.store(value, Ordering::SeqCst);
    set_leader_status(value);
    let _ = tx.send(value);
}

async fn try_acquire_or_renew(
    leases: &Api<Lease>,
    lease_name: &str,
    namespace: &str,
    identity: &str,
) -> Result<bool, LeaseError> {
    let now = Utc::now();

    match bounded_api(leases.get(lease_name)).await {
        Ok(mut existing) => {
            let spec = existing.spec.get_or_insert_with(LeaseSpec::default);
            let current_holder = spec.holder_identity.as_deref();
            if current_holder != Some(identity)
                && current_holder.is_some()
                && !lease_expired(Some(spec), now)
            {
                return Ok(false);
            }

            if current_holder != Some(identity) {
                spec.acquire_time = Some(MicroTime(now));
            }
            spec.holder_identity = Some(identity.to_string());
            spec.renew_time = Some(MicroTime(now));
            spec.lease_duration_seconds = Some(LEASE_DURATION_SECS);

            match bounded_api(leases.replace(lease_name, &PostParams::default(), &existing)).await {
                Ok(_) => Ok(true),
                Err(LeaseError::Kubernetes(error)) if is_conflict(&error) => Ok(false),
                Err(error) => Err(error),
            }
        }
        Err(LeaseError::Kubernetes(kube::Error::Api(error))) if error.code == 404 => {
            let lease = Lease {
                metadata: ObjectMeta {
                    name: Some(lease_name.to_string()),
                    namespace: Some(namespace.to_string()),
                    ..Default::default()
                },
                spec: Some(LeaseSpec {
                    holder_identity: Some(identity.to_string()),
                    acquire_time: Some(MicroTime(now)),
                    renew_time: Some(MicroTime(now)),
                    lease_duration_seconds: Some(LEASE_DURATION_SECS),
                    ..Default::default()
                }),
            };
            match bounded_api(leases.create(&PostParams::default(), &lease)).await {
                Ok(_) => Ok(true),
                Err(LeaseError::Kubernetes(error)) if is_conflict(&error) => Ok(false),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

async fn bounded_api<T>(
    request: impl std::future::Future<Output = Result<T, kube::Error>>,
) -> Result<T, LeaseError> {
    tokio::time::timeout(API_TIMEOUT, request)
        .await
        .map_err(|_| LeaseError::Timeout)?
        .map_err(LeaseError::Kubernetes)
}

fn is_conflict(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(response) if response.code == 409)
}

fn default_lease_namespace() -> String {
    std::env::var("OPERATOR_LEASE_NAMESPACE")
        .or_else(|_| std::env::var("POD_NAMESPACE"))
        .unwrap_or_else(|_| "default".to_string())
}

fn pod_identity() -> String {
    std::env::var("POD_UID")
        .or_else(|_| std::env::var("POD_NAME"))
        .or_else(|_| {
            hostname::get().map(|host| host.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|_| format!("unknown-{}", std::process::id()))
}

fn lease_expired(spec: Option<&LeaseSpec>, now: chrono::DateTime<Utc>) -> bool {
    spec.and_then(|s| s.renew_time.as_ref())
        .map(|renew| {
            let duration = spec
                .and_then(|s| s.lease_duration_seconds)
                .unwrap_or(LEASE_DURATION_SECS);
            let expiry = renew.0 + chrono::Duration::seconds(duration as i64);
            now >= expiry
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_expiry_uses_server_renewal_time_and_ttl() {
        let now = Utc::now();
        let spec = LeaseSpec {
            renew_time: Some(MicroTime(now - chrono::Duration::seconds(15))),
            lease_duration_seconds: Some(15),
            holder_identity: Some("old-holder".to_string()),
            ..Default::default()
        };
        assert!(lease_expired(Some(&spec), now));
    }

    #[test]
    fn lease_is_not_expired_before_ttl() {
        let now = Utc::now();
        let spec = LeaseSpec {
            renew_time: Some(MicroTime(now - chrono::Duration::seconds(14))),
            lease_duration_seconds: Some(15),
            holder_identity: Some("current-holder".to_string()),
            ..Default::default()
        };
        assert!(!lease_expired(Some(&spec), now));
    }

    #[test]
    fn lease_without_renewal_time_is_available() {
        assert!(lease_expired(Some(&LeaseSpec::default()), Utc::now()));
    }
}

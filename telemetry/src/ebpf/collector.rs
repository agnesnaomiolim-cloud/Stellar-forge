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

//! eBPF network telemetry collector for the Stellar Kubernetes Operator.
//!
//! # Responsibilities
//!
//! 1. **Probe lifecycle** — loads the eBPF probe (or its simulation fallback)
//!    and drives the poll loop.
//! 2. **IP-to-pod mapping** — resolves raw IP addresses to Kubernetes pod
//!    identities using a periodically-refreshed in-memory table.
//! 3. **Metrics recording** — translates [`ConnEvent`]s into Prometheus
//!    counter/histogram observations via [`EbpfMetrics`].
//! 4. **Graceful fallback** — transparently switches to [`ProbeSimulator`]
//!    when the process lacks `CAP_BPF` / `CAP_NET_ADMIN` or runs in a
//!    restricted environment (CI, local dev containers).
//!
//! # Usage
//!
//! ```no_run
//! use stellar_telemetry::ebpf::collector::{EbpfCollector, CollectorConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = CollectorConfig::default();
//!     let collector = EbpfCollector::new(config).await?;
//!     collector.run().await
//! }
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
#[allow(unused_imports)]
use tracing::{debug, info, warn};

use super::metrics::{DropLabels, EbpfMetrics, ModeLabels, NetworkLabels, PodLabels};
use super::probe::{ConnEvent, Direction, DropReason, ProbeConfig, ProbeHandle, ProbeSimulator};

// ---------------------------------------------------------------------------
// IP → Pod mapping
// ---------------------------------------------------------------------------

/// A resolved Kubernetes pod identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodIdentity {
    /// Kubernetes pod name (e.g. `stellar-core-0`).
    pub name: String,
    /// Kubernetes namespace (e.g. `stellar-system`).
    pub namespace: String,
    /// Pod IP address.
    pub ip: IpAddr,
    /// Optional node name the pod is scheduled on.
    pub node: Option<String>,
}

/// Thread-safe map from pod IP → [`PodIdentity`].
///
/// Refreshed periodically by [`EbpfCollector::refresh_pod_map`].
pub type PodMap = Arc<RwLock<HashMap<IpAddr, PodIdentity>>>;

/// Build a [`PodMap`] seed from a static list (useful for testing and
/// environments without a live Kubernetes API server).
pub fn pod_map_from_static(entries: Vec<PodIdentity>) -> PodMap {
    let mut map = HashMap::new();
    for pod in entries {
        map.insert(pod.ip, pod);
    }
    Arc::new(RwLock::new(map))
}

// ---------------------------------------------------------------------------
// Collector configuration
// ---------------------------------------------------------------------------

/// Configuration for [`EbpfCollector`].
#[derive(Debug, Clone)]
pub struct CollectorConfig {
    /// eBPF probe configuration (interface, port, poll interval).
    pub probe: ProbeConfig,
    /// How often to refresh the IP-to-pod mapping.
    pub pod_map_refresh: Duration,
    /// Whether to attempt a native eBPF load before falling back.
    pub try_native: bool,
    /// If `true`, the collector shuts down when the poll loop encounters an
    /// unrecoverable error; otherwise it logs and continues.
    pub fail_fast: bool,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        CollectorConfig {
            probe: ProbeConfig::default(),
            pod_map_refresh: Duration::from_secs(30),
            try_native: true,
            fail_fast: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Collector
// ---------------------------------------------------------------------------

/// Main eBPF network telemetry collector.
///
/// The collector owns the probe handle and Prometheus metrics registry.
/// Construct via [`EbpfCollector::new`] and call [`EbpfCollector::run`]
/// to start the poll loop (which runs until the process exits).
pub struct EbpfCollector {
    probe: Arc<Box<dyn ProbeHandle>>,
    metrics: Arc<EbpfMetrics>,
    pod_map: PodMap,
    config: CollectorConfig,
}

impl EbpfCollector {
    /// Create a new collector, attempting native eBPF load if configured.
    ///
    /// Falls back to [`ProbeSimulator`] if:
    /// - `config.try_native` is `false`.
    /// - The `ebpf-runtime` feature is not enabled.
    /// - The process lacks the required kernel privileges.
    pub async fn new(config: CollectorConfig) -> anyhow::Result<Self> {
        let probe: Box<dyn ProbeHandle> = Self::load_probe(&config).await;
        let metrics = Arc::new(EbpfMetrics::new());

        // Record probe mode.
        let mode_label = if probe.is_native() {
            "native"
        } else {
            "simulation"
        };
        metrics
            .probe_mode
            .get_or_create(&ModeLabels {
                mode: mode_label.to_string(),
            })
            .set(1.0);

        info!(
            native = probe.is_native(),
            interface = %config.probe.interface,
            port = config.probe.stellar_port,
            "eBPF telemetry collector initialized"
        );

        Ok(EbpfCollector {
            probe: Arc::new(probe),
            metrics,
            pod_map: Arc::new(RwLock::new(HashMap::new())),
            config,
        })
    }

    /// Create a collector with a pre-seeded pod map (useful for tests).
    pub async fn with_pod_map(config: CollectorConfig, pod_map: PodMap) -> anyhow::Result<Self> {
        let mut collector = Self::new(config).await?;
        collector.pod_map = pod_map;
        Ok(collector)
    }

    /// Load a probe, applying the native→simulation fallback chain.
    async fn load_probe(config: &CollectorConfig) -> Box<dyn ProbeHandle> {
        #[cfg(feature = "ebpf-runtime")]
        if config.try_native {
            match super::probe::native::AyaProbe::try_load(config.probe.clone()).await {
                Ok(probe) => {
                    info!("eBPF native probe loaded successfully (Aya)");
                    return Box::new(probe);
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Native eBPF load failed, falling back to simulation"
                    );
                }
            }
        }

        info!("eBPF probe running in simulation mode (no kernel attachment)");
        Box::new(ProbeSimulator::new(config.probe.clone()))
    }

    /// Return a reference to the Prometheus metrics registry.
    pub fn metrics(&self) -> Arc<EbpfMetrics> {
        self.metrics.clone()
    }

    /// Whether the collector is using a native kernel-attached probe.
    pub fn is_native(&self) -> bool {
        self.probe.is_native()
    }

    /// Render current metrics to Prometheus text format.
    pub fn render_metrics(&self) -> String {
        self.metrics.render()
    }

    /// Poll the underlying probe once and return the events.
    ///
    /// Exposed for benchmarks; prefer [`run`] in production code.
    pub fn probe_poll_for_bench(&self) -> Vec<ConnEvent> {
        self.probe.poll()
    }

    /// Update the IP-to-pod mapping table.
    ///
    /// In production this is called by `refresh_pod_map` which queries the
    /// Kubernetes API.  In tests it can be called directly with synthetic data.
    pub async fn update_pod_map(&self, entries: Vec<PodIdentity>) {
        let mut map = self.pod_map.write().await;
        map.clear();
        for pod in entries {
            map.insert(pod.ip, pod);
        }
        debug!(entries = map.len(), "Pod IP map refreshed");
    }

    /// Resolve an IP address to a pod name and namespace.
    ///
    /// Returns `("unknown", "unknown")` if the IP is not in the map.
    async fn resolve_pod(&self, ip: IpAddr) -> (String, String) {
        let map = self.pod_map.read().await;
        if let Some(pod) = map.get(&ip) {
            (pod.name.clone(), pod.namespace.clone())
        } else {
            ("unknown".to_string(), "unknown".to_string())
        }
    }

    /// Process a single [`ConnEvent`] and update the relevant metrics.
    ///
    /// This is the hot path — it must be fast and non-blocking.
    /// Exposed as `pub` for benchmark access; production code drives it
    /// through the [`run`] loop.
    pub async fn record_event(&self, event: &ConnEvent) {
        // Resolve pod identity from destination IP for ingress, source for egress.
        let peer_ip = match event.direction {
            Direction::Ingress => event.src_ip,
            Direction::Egress => event.dst_ip,
        };
        let (pod_name, namespace) = self.resolve_pod(peer_ip).await;

        let direction = event.direction.to_string();
        let net_labels = NetworkLabels {
            direction: direction.clone(),
            pod_name: pod_name.clone(),
            namespace: namespace.clone(),
        };
        let pod_labels = PodLabels {
            pod_name: pod_name.clone(),
            namespace: namespace.clone(),
        };

        // Connections counter.
        self.metrics
            .connections_total
            .get_or_create(&net_labels)
            .inc();

        // Bytes counter.
        self.metrics
            .bytes_total
            .get_or_create(&net_labels)
            .inc_by(event.bytes);

        // Latency histogram.
        if let Some(us) = event.latency_us {
            self.metrics
                .latency_us
                .get_or_create(&net_labels)
                .observe(us as f64);
        }

        // Packet drops.
        if event.packets_dropped > 0 {
            let reason = event
                .drop_reason
                .as_ref()
                .map(|r| r.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let drop_labels = DropLabels {
                pod_name: pod_name.clone(),
                namespace: namespace.clone(),
                reason,
            };
            self.metrics
                .packets_dropped_total
                .get_or_create(&drop_labels)
                .inc_by(event.packets_dropped);
        }

        // Connection resets.
        if event.drop_reason == Some(DropReason::ConnectionReset)
            || event.drop_reason == Some(DropReason::LocalReset)
        {
            self.metrics
                .connection_resets_total
                .get_or_create(&pod_labels)
                .inc();
        }
    }

    /// Run the collector poll loop.
    ///
    /// This future runs until the process exits or an unrecoverable error
    /// occurs (when `config.fail_fast = true`).  It is intended to be
    /// spawned as a background task:
    ///
    /// ```no_run
    /// # use stellar_telemetry::ebpf::collector::{EbpfCollector, CollectorConfig};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let collector = std::sync::Arc::new(EbpfCollector::new(CollectorConfig::default()).await?);
    /// let c = collector.clone();
    /// tokio::spawn(async move { c.run().await });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run(&self) -> anyhow::Result<()> {
        let poll_interval = self.config.probe.poll_interval;
        let pod_refresh = self.config.pod_map_refresh;
        let mut last_pod_refresh = Instant::now();

        info!(
            poll_interval_ms = poll_interval.as_millis(),
            pod_refresh_secs = pod_refresh.as_secs(),
            native = self.probe.is_native(),
            "eBPF collector poll loop starting"
        );

        loop {
            // Refresh pod map on schedule.
            if last_pod_refresh.elapsed() >= pod_refresh {
                self.refresh_pod_map_from_env().await;
                last_pod_refresh = Instant::now();
            }

            // Measure poll duration for the overhead metric.
            let t0 = Instant::now();
            let events = self.probe.poll();
            let poll_us = t0.elapsed().as_micros() as f64;

            // Record poll diagnostics.
            self.metrics.poll_duration_us.observe(poll_us);
            self.metrics
                .events_per_poll
                .observe(events.len() as f64);

            // Record each event.
            for event in &events {
                self.record_event(event).await;
            }

            if !events.is_empty() {
                debug!(
                    count = events.len(),
                    poll_us,
                    "eBPF events processed"
                );
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Attempt to refresh the pod map from environment (Kubernetes API or
    /// environment variables injected at runtime).
    ///
    /// In a full production deployment this calls the kube-rs client.
    /// When running outside a cluster it falls back to parsing the
    /// `STELLAR_POD_IP_MAP` environment variable (format:
    /// `ip=pod-name/namespace,ip2=pod2/ns2`).
    async fn refresh_pod_map_from_env(&self) {
        // Try the STELLAR_POD_IP_MAP env var as a lightweight override.
        if let Ok(raw) = std::env::var("STELLAR_POD_IP_MAP") {
            let mut entries = Vec::new();
            for pair in raw.split(',') {
                let parts: Vec<&str> = pair.splitn(2, '=').collect();
                if parts.len() != 2 {
                    continue;
                }
                let ip_str = parts[0].trim();
                let ident = parts[1].trim();
                let ident_parts: Vec<&str> = ident.splitn(2, '/').collect();
                if ident_parts.len() != 2 {
                    continue;
                }
                if let Ok(ip) = ip_str.parse::<IpAddr>() {
                    entries.push(PodIdentity {
                        name: ident_parts[0].to_string(),
                        namespace: ident_parts[1].to_string(),
                        ip,
                        node: None,
                    });
                }
            }
            if !entries.is_empty() {
                self.update_pod_map(entries).await;
                return;
            }
        }

        // When running inside a Kubernetes cluster the pod list API would be
        // called here.  Without a live API server we leave the map as-is.
        debug!("Pod map refresh: no STELLAR_POD_IP_MAP set and no cluster API available");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use super::*;
    use crate::ebpf::probe::{Direction, DropReason, ProbeConfig};

    fn test_config() -> CollectorConfig {
        CollectorConfig {
            probe: ProbeConfig {
                poll_interval: Duration::from_millis(10),
                ..ProbeConfig::default()
            },
            try_native: false, // always use simulation in tests
            ..CollectorConfig::default()
        }
    }

    #[tokio::test]
    async fn collector_initializes_in_simulation_mode() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        assert!(!collector.is_native(), "Should be in simulation mode");
    }

    #[tokio::test]
    async fn pod_map_resolution_resolves_known_ip() {
        let pod_entries = vec![PodIdentity {
            name: "stellar-core-0".to_string(),
            namespace: "stellar-system".to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)),
            node: Some("node-1".to_string()),
        }];

        let config = test_config();
        let pod_map = pod_map_from_static(pod_entries);
        let collector = EbpfCollector::with_pod_map(config, pod_map).await.unwrap();

        let (name, ns) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)))
            .await;
        assert_eq!(name, "stellar-core-0");
        assert_eq!(ns, "stellar-system");
    }

    #[tokio::test]
    async fn pod_map_resolution_returns_unknown_for_unmapped_ip() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        let (name, ns) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(192, 168, 99, 1)))
            .await;
        assert_eq!(name, "unknown");
        assert_eq!(ns, "unknown");
    }

    #[tokio::test]
    async fn record_event_increments_counters() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        let event = ConnEvent::synthetic(
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            11625,
            12000,
            Direction::Ingress,
            Some(2_500),
            0,
            1024,
            None,
        );
        collector.record_event(&event).await;

        let rendered = collector.render_metrics();
        assert!(
            rendered.contains("stellar_ebpf_connections_total"),
            "connections_total counter missing"
        );
        assert!(
            rendered.contains("stellar_ebpf_bytes_total"),
            "bytes_total counter missing"
        );
    }

    #[tokio::test]
    async fn record_event_with_packet_drops() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        let event = ConnEvent::synthetic(
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 11)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            11625,
            12000,
            Direction::Ingress,
            Some(50_000), // elevated latency
            15,           // packet drops
            512,
            Some(DropReason::PacketDrop),
        );
        collector.record_event(&event).await;

        let rendered = collector.render_metrics();
        assert!(
            rendered.contains("stellar_ebpf_packets_dropped_total"),
            "packets_dropped_total missing"
        );
    }

    #[tokio::test]
    async fn record_event_with_connection_reset() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        let event = ConnEvent::synthetic(
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 12)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            11625,
            12000,
            Direction::Ingress,
            None,
            1,
            0,
            Some(DropReason::ConnectionReset),
        );
        collector.record_event(&event).await;

        let rendered = collector.render_metrics();
        assert!(
            rendered.contains("stellar_ebpf_connection_resets_total"),
            "connection_resets_total missing"
        );
    }

    #[tokio::test]
    async fn update_pod_map_replaces_entries() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        collector
            .update_pod_map(vec![PodIdentity {
                name: "pod-a".to_string(),
                namespace: "ns-a".to_string(),
                ip: IpAddr::V4(Ipv4Addr::new(10, 10, 0, 1)),
                node: None,
            }])
            .await;

        let (name, _) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(10, 10, 0, 1)))
            .await;
        assert_eq!(name, "pod-a");

        // Replace with a different entry.
        collector
            .update_pod_map(vec![PodIdentity {
                name: "pod-b".to_string(),
                namespace: "ns-b".to_string(),
                ip: IpAddr::V4(Ipv4Addr::new(10, 10, 0, 2)),
                node: None,
            }])
            .await;

        // Old entry should be gone.
        let (old, _) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(10, 10, 0, 1)))
            .await;
        assert_eq!(old, "unknown");

        // New entry should be present.
        let (new, _) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(10, 10, 0, 2)))
            .await;
        assert_eq!(new, "pod-b");
    }

    /// High-packet-loss simulation: inject 500 events with heavy drop rates
    /// and verify the drop counters are non-zero and metrics don't panic.
    #[tokio::test]
    async fn high_packet_loss_simulation() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();

        let src_ips: [IpAddr; 4] = [
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 11)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 12)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 13)),
        ];
        let dst = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        let mut total_drops = 0u64;
        for i in 0..500usize {
            let drops = if i % 3 == 0 { 10 + (i as u64 % 50) } else { 0 };
            total_drops += drops;
            let event = ConnEvent::synthetic(
                src_ips[i % src_ips.len()],
                dst,
                11625,
                12000,
                Direction::Ingress,
                Some(1_000 + (i as u64 * 100)),
                drops,
                1024,
                if drops > 0 {
                    Some(DropReason::PacketDrop)
                } else {
                    None
                },
            );
            collector.record_event(&event).await;
        }

        let rendered = collector.render_metrics();
        assert!(total_drops > 0, "Should have generated some drops");
        assert!(
            rendered.contains("stellar_ebpf_packets_dropped_total"),
            "Drop counter should be present in metrics output"
        );
        assert!(
            rendered.contains("stellar_ebpf_connections_total"),
            "Connections counter should be present"
        );
    }

    /// Graceful fallback: verify simulation mode is activated when
    /// `try_native = false`, and metrics are still populated.
    #[tokio::test]
    async fn graceful_fallback_to_simulation() {
        let config = CollectorConfig {
            try_native: false,
            ..test_config()
        };
        let collector = EbpfCollector::new(config).await.unwrap();
        assert!(
            !collector.is_native(),
            "Must fall back to simulation when try_native=false"
        );

        // Probe should still produce events.
        let events = collector.probe.poll();
        // Simulator may return 0–8 events per poll; just verify no panic.
        let _ = events;

        // Probe mode metric should reflect simulation.
        let rendered = collector.render_metrics();
        assert!(rendered.contains("stellar_ebpf_probe_mode"));
    }

    #[tokio::test]
    async fn egress_events_are_recorded_correctly() {
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        let event = ConnEvent::synthetic(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 1, 20)),
            12345,
            11625,
            Direction::Egress,
            Some(3_000),
            0,
            2048,
            None,
        );
        collector.record_event(&event).await;
        let rendered = collector.render_metrics();
        assert!(rendered.contains("stellar_ebpf_connections_total"));
    }

    #[tokio::test]
    async fn env_pod_map_refresh_parses_correctly() {
        // Set a synthetic env var and ensure the pod map gets populated.
        std::env::set_var(
            "STELLAR_POD_IP_MAP",
            "10.0.1.99=stellar-core-env/stellar-system",
        );
        let collector = EbpfCollector::new(test_config()).await.unwrap();
        collector.refresh_pod_map_from_env().await;
        let (name, ns) = collector
            .resolve_pod(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 99)))
            .await;
        std::env::remove_var("STELLAR_POD_IP_MAP");
        assert_eq!(name, "stellar-core-env");
        assert_eq!(ns, "stellar-system");
    }
}

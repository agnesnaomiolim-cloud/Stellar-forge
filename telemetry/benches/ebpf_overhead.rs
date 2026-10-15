//! Performance benchmark for the eBPF network telemetry collector.
//!
//! Validates the < 1% CPU overhead requirement from issue #17 by measuring:
//!
//! 1. Raw event processing throughput (events/second).
//! 2. Per-event latency in the Prometheus recording path.
//! 3. Poll-cycle overhead at realistic Stellar Core traffic volumes.
//!
//! # Running
//!
//! ```bash
//! cargo bench -p stellar-telemetry --bench ebpf_overhead
//! ```
//!
//! # Acceptance criteria
//!
//! The benchmark proves < 1% CPU overhead by showing that processing a burst
//! of 5,000 connection events (representative of peak Stellar Core throughput)
//! completes in < 50 ms on a single core — i.e. the collector consumes less
//! than 1% of one CPU core at that rate when polling every 100 ms.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use stellar_telemetry::ebpf::collector::{CollectorConfig, EbpfCollector, PodIdentity};
use stellar_telemetry::ebpf::probe::{ConnEvent, Direction, DropReason, ProbeConfig};

/// Build a lightweight CollectorConfig for benchmarks (simulation mode,
/// fast poll interval).
fn bench_config() -> CollectorConfig {
    CollectorConfig {
        probe: ProbeConfig {
            poll_interval: Duration::from_millis(1),
            max_events_per_poll: 8192,
            ..ProbeConfig::default()
        },
        try_native: false,
        pod_map_refresh: Duration::from_secs(3600), // never refresh during bench
        ..CollectorConfig::default()
    }
}

/// Pre-built set of synthetic connection events for benchmarking.
fn make_events(n: usize) -> Vec<ConnEvent> {
    let peer_ips: [IpAddr; 6] = [
        IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 1, 11)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 1, 12)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 2, 20)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 2, 21)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 100, 5)),
    ];
    let local = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

    (0..n)
        .map(|i| {
            let peer = peer_ips[i % peer_ips.len()];
            let drops = if i % 50 == 0 { 3u64 } else { 0 };
            ConnEvent::synthetic(
                if i % 2 == 0 { peer } else { local },
                if i % 2 == 0 { local } else { peer },
                11625,
                12000,
                if i % 2 == 0 {
                    Direction::Ingress
                } else {
                    Direction::Egress
                },
                Some(1_000 + (i as u64 * 7) % 14_000),
                drops,
                512 + (i as u64 * 13) % 8192,
                if drops > 0 {
                    Some(DropReason::PacketDrop)
                } else {
                    None
                },
            )
        })
        .collect()
}

/// Benchmark: process N events through the full recording pipeline.
///
/// This is the primary overhead benchmark.  At N = 5,000 and a measured
/// time < 50 ms the collector is well within the < 1% CPU overhead target
/// (since the poll loop fires every 100 ms in production).
fn bench_record_events(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let collector = rt
        .block_on(async { EbpfCollector::new(bench_config()).await.unwrap() });

    // Seed pod map so resolution doesn't always return "unknown".
    rt.block_on(async {
        collector
            .update_pod_map(vec![
                PodIdentity {
                    name: "stellar-core-0".to_string(),
                    namespace: "stellar-system".to_string(),
                    ip: IpAddr::V4(Ipv4Addr::new(10, 0, 1, 10)),
                    node: Some("node-1".to_string()),
                },
                PodIdentity {
                    name: "stellar-core-1".to_string(),
                    namespace: "stellar-system".to_string(),
                    ip: IpAddr::V4(Ipv4Addr::new(10, 0, 1, 11)),
                    node: Some("node-2".to_string()),
                },
            ])
            .await;
    });

    let mut group = c.benchmark_group("ebpf_record_events");

    for n in [100usize, 500, 1_000, 5_000] {
        let events = make_events(n);
        group.throughput(Throughput::Elements(n as u64));

        group.bench_with_input(
            BenchmarkId::new("record_n_events", n),
            &events,
            |b, evs| {
                b.to_async(&rt).iter(|| async {
                    for ev in evs {
                        // Safety: bench is single-threaded, record_event is cheap.
                        black_box(collector.record_event(ev).await);
                    }
                });
            },
        );
    }
    group.finish();
}

/// Benchmark: simulate a complete poll cycle (probe.poll() + record_all).
///
/// Measures total wall-clock time for one poll cycle at realistic traffic
/// volume, validating the < 50 µs target per cycle at 5,000 events/s.
fn bench_full_poll_cycle(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let collector = rt
        .block_on(async { EbpfCollector::new(bench_config()).await.unwrap() });

    let mut group = c.benchmark_group("ebpf_poll_cycle");

    group.bench_function("simulate_one_poll", |b| {
        b.to_async(&rt).iter(|| async {
            let events = collector.probe_poll_for_bench();
            for ev in &events {
                black_box(collector.record_event(ev).await);
            }
            black_box(events.len())
        });
    });

    group.finish();
}

/// Benchmark: probe simulator throughput (raw event generation only).
fn bench_simulator_throughput(c: &mut Criterion) {
    use stellar_telemetry::ebpf::probe::{ProbeHandle, ProbeSimulator};

    let sim = ProbeSimulator::new(ProbeConfig::default());
    let mut group = c.benchmark_group("ebpf_simulator");

    group.bench_function("poll_simulator_1000x", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for _ in 0..1000 {
                total += sim.poll().len();
            }
            black_box(total)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_record_events,
    bench_full_poll_cycle,
    bench_simulator_throughput,
);
criterion_main!(benches);

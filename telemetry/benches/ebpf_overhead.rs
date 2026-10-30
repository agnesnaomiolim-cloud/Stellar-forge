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
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use stellar_telemetry::ebpf::collector::{simulate_loss_workload, CollectorMode, EbpfCollector};
use std::net::{IpAddr, Ipv4Addr};

fn bench_ingest_50k(c: &mut Criterion) {
    c.bench_function("ebpf_ingest_50k_samples", |b| {
        b.iter(|| {
            let collector = EbpfCollector::new(CollectorMode::Ebpf);
            collector.watch_stellar_core(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1)));
            simulate_loss_workload(&collector, black_box(50_000), 0.01, 0.0);
        });
    });
}

criterion_group!(benches, bench_ingest_50k);
criterion_main!(benches);
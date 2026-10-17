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

#[path = "../src/ebpf/mod.rs"]
mod ebpf;

use ebpf::collector::{
    EbpfNetworkCollector, EbpfRuntimeMode, SocketTraceEvent, STELLAR_P2P_OVERLAY_PORT,
};

#[test]
fn test_ebpf_collector_filters_overlay_port_and_gracefully_degrades_on_old_kernel() {
    // 1. Kernel 5.4 without eBPF ringbuf -> degrades gracefully to UserspaceProcNetFallback
    let fallback = EbpfNetworkCollector::initialize(5, 4, true);
    assert_eq!(fallback.mode, EbpfRuntimeMode::UserspaceProcNetFallback);

    // 2. Kernel 6.1 with CAP_BPF -> attaches AyaKernelRingBuf
    let mut collector = EbpfNetworkCollector::initialize(6, 1, true);
    assert_eq!(collector.mode, EbpfRuntimeMode::AyaKernelRingBuf);

    // 3. Non-P2P traffic is ignored
    let non_p2p = SocketTraceEvent {
        peer_addr: "10.0.1.9:443".to_string(),
        dst_port: 443,
        rtt_micros: 500,
        bytes_sent: 1024,
        bytes_received: 2048,
        retransmissions: 0,
        dropped_packets: 0,
    };
    assert!(!collector.ingest_trace_event(&non_p2p));

    // 4. Stellar P2P Overlay port (11625) traffic is aggregated & exported to Prometheus
    let p2p_ev = SocketTraceEvent {
        peer_addr: "10.244.1.15:11625".to_string(),
        dst_port: STELLAR_P2P_OVERLAY_PORT,
        rtt_micros: 1_850,
        bytes_sent: 65_536,
        bytes_received: 131_072,
        retransmissions: 2,
        dropped_packets: 1,
    };
    assert!(collector.ingest_trace_event(&p2p_ev));

    let prom = collector.render_prometheus_metrics();
    assert!(prom.contains("stellar_ebpf_p2p_rtt_microseconds{peer=\"10.244.1.15:11625\"} 1850"));
    assert!(prom.contains("stellar_ebpf_tcp_retransmissions_total{peer=\"10.244.1.15:11625\"} 2"));
}

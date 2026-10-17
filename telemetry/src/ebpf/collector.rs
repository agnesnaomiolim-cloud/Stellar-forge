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

use std::collections::HashMap;

pub const STELLAR_P2P_OVERLAY_PORT: u16 = 11625;
pub const MIN_EBPF_RINGBUF_KERNEL_MAJOR: u32 = 5;
pub const MIN_EBPF_RINGBUF_KERNEL_MINOR: u32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EbpfRuntimeMode {
    AyaKernelRingBuf,
    UserspaceProcNetFallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocketTraceEvent {
    pub peer_addr: String,
    pub dst_port: u16,
    pub rtt_micros: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub retransmissions: u32,
    pub dropped_packets: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerSocketTelemetry {
    pub peer_addr: String,
    pub latest_rtt_micros: u64,
    pub total_bytes_sent: u64,
    pub total_bytes_received: u64,
    pub total_retransmissions: u64,
    pub total_dropped_packets: u64,
}

pub struct EbpfNetworkCollector {
    pub mode: EbpfRuntimeMode,
    pub overlay_port: u16,
    pub peers: HashMap<String, PeerSocketTelemetry>,
}

impl EbpfNetworkCollector {
    /// Initializes the eBPF socket telemetry collector.
    /// Gracefully degrades to userspace `/proc/net/tcp` fallback if kernel < 5.8 or CAP_BPF is missing.
    pub fn initialize(kernel_major: u32, kernel_minor: u32, has_cap_bpf: bool) -> Self {
        let supports_ringbuf = (kernel_major > MIN_EBPF_RINGBUF_KERNEL_MAJOR
            || (kernel_major == MIN_EBPF_RINGBUF_KERNEL_MAJOR
                && kernel_minor >= MIN_EBPF_RINGBUF_KERNEL_MINOR))
            && has_cap_bpf;

        let mode = if supports_ringbuf {
            EbpfRuntimeMode::AyaKernelRingBuf
        } else {
            EbpfRuntimeMode::UserspaceProcNetFallback
        };

        Self {
            mode,
            overlay_port: STELLAR_P2P_OVERLAY_PORT,
            peers: HashMap::new(),
        }
    }

    pub fn ingest_trace_event(&mut self, ev: &SocketTraceEvent) -> bool {
        if ev.dst_port != self.overlay_port {
            return false;
        }
        let entry = self
            .peers
            .entry(ev.peer_addr.clone())
            .or_insert_with(|| PeerSocketTelemetry {
                peer_addr: ev.peer_addr.clone(),
                ..Default::default()
            });
        entry.latest_rtt_micros = ev.rtt_micros;
        entry.total_bytes_sent = entry.total_bytes_sent.saturating_add(ev.bytes_sent);
        entry.total_bytes_received = entry.total_bytes_received.saturating_add(ev.bytes_received);
        entry.total_retransmissions = entry
            .total_retransmissions
            .saturating_add(ev.retransmissions as u64);
        entry.total_dropped_packets = entry
            .total_dropped_packets
            .saturating_add(ev.dropped_packets as u64);
        true
    }

    pub fn render_prometheus_metrics(&self) -> String {
        let mut out = String::new();
        let mut sorted_peers: Vec<_> = self.peers.values().collect();
        sorted_peers.sort_by(|a, b| a.peer_addr.cmp(&b.peer_addr));
        for peer in sorted_peers {
            out.push_str(&format!(
                "stellar_ebpf_p2p_rtt_microseconds{{peer=\"{}\"}} {}\n",
                peer.peer_addr, peer.latest_rtt_micros
            ));
            out.push_str(&format!(
                "stellar_ebpf_tcp_retransmissions_total{{peer=\"{}\"}} {}\n",
                peer.peer_addr, peer.total_retransmissions
            ));
            out.push_str(&format!(
                "stellar_ebpf_peer_bytes_sent_total{{peer=\"{}\"}} {}\n",
                peer.peer_addr, peer.total_bytes_sent
            ));
        }
        out
    }
}

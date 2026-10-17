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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScpStage {
    Nominate,
    Prepare,
    Commit,
    Externalize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScpEnvelopeEvent {
    pub ledger_sequence: u64,
    pub validator_node_id: String,
    pub stage: ScpStage,
    pub raw_xdr_base64: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartitionDispatchOutcome {
    Routed {
        partition_id: u32,
        ledger_sequence: u64,
    },
    SentToDeadLetterQueue {
        reason: String,
    },
    BackpressureThrottled {
        partition_id: u32,
    },
}

pub struct ScpKafkaPartitioner {
    pub num_partitions: u32,
    pub max_in_flight_per_partition: usize,
    pub in_flight_counts: Vec<usize>,
    pub dlq_events: Vec<(String, String)>,
}

impl ScpKafkaPartitioner {
    pub fn new(num_partitions: u32, max_in_flight_per_partition: usize) -> Self {
        let count = num_partitions.max(1);
        Self {
            num_partitions: count,
            max_in_flight_per_partition: max_in_flight_per_partition.max(1),
            in_flight_counts: vec![0; count as usize],
            dlq_events: Vec::new(),
        }
    }

    /// Deterministically hashes `ledger_sequence` using FNV-1a 64-bit so that all
    /// `NOMINATE`, `PREPARE`, `COMMIT`, and `EXTERNALIZE` messages for a given ledger
    /// are guaranteed to land on the exact same Kafka partition in causal order.
    pub fn partition_for_ledger(&self, ledger_sequence: u64) -> u32 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in ledger_sequence.to_be_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        (hash % (self.num_partitions as u64)) as u32
    }

    pub fn dispatch(&mut self, event: &ScpEnvelopeEvent) -> PartitionDispatchOutcome {
        if event.raw_xdr_base64.trim().is_empty()
            || event.raw_xdr_base64.len() % 4 != 0
            || event.validator_node_id.trim().is_empty()
        {
            let reason = "malformed_scp_xdr_envelope".to_string();
            self.dlq_events
                .push((event.validator_node_id.clone(), reason.clone()));
            return PartitionDispatchOutcome::SentToDeadLetterQueue { reason };
        }

        let partition_id = self.partition_for_ledger(event.ledger_sequence);
        let idx = partition_id as usize;
        if self.in_flight_counts[idx] >= self.max_in_flight_per_partition {
            return PartitionDispatchOutcome::BackpressureThrottled { partition_id };
        }

        self.in_flight_counts[idx] += 1;
        PartitionDispatchOutcome::Routed {
            partition_id,
            ledger_sequence: event.ledger_sequence,
        }
    }

    pub fn ack_partition_delivery(&mut self, partition_id: u32) {
        if let Some(slot) = self.in_flight_counts.get_mut(partition_id as usize) {
            *slot = slot.saturating_sub(1);
        }
    }
}

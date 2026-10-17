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

#[path = "../src/scp_stream.rs"]
mod scp_stream;

use scp_stream::{PartitionDispatchOutcome, ScpEnvelopeEvent, ScpKafkaPartitioner, ScpStage};

#[test]
fn test_scp_kafka_partitioner_locality_backpressure_and_dlq() {
    let mut partitioner = ScpKafkaPartitioner::new(12, 2);

    // 1. All SCP stages for ledger 50_000_123 route to the exact same Kafka partition
    let p_nominate = partitioner.dispatch(&ScpEnvelopeEvent {
        ledger_sequence: 50_000_123,
        validator_node_id: "GVALIDATOR1".to_string(),
        stage: ScpStage::Nominate,
        raw_xdr_base64: "AAAAAgAAAAM=".to_string(),
    });
    let p_externalize = partitioner.dispatch(&ScpEnvelopeEvent {
        ledger_sequence: 50_000_123,
        validator_node_id: "GVALIDATOR2".to_string(),
        stage: ScpStage::Externalize,
        raw_xdr_base64: "BBBBBgAAAAM=".to_string(),
    });

    let expected_part = partitioner.partition_for_ledger(50_000_123);
    assert_eq!(
        p_nominate,
        PartitionDispatchOutcome::Routed {
            partition_id: expected_part,
            ledger_sequence: 50_000_123
        }
    );
    assert_eq!(p_nominate, p_externalize);

    // 2. Exceeding max_in_flight_per_partition (2) triggers non-blocking backpressure
    let p_overflow = partitioner.dispatch(&ScpEnvelopeEvent {
        ledger_sequence: 50_000_123,
        validator_node_id: "GVALIDATOR3".to_string(),
        stage: ScpStage::Commit,
        raw_xdr_base64: "CCCCBgAAAAM=".to_string(),
    });
    assert_eq!(
        p_overflow,
        PartitionDispatchOutcome::BackpressureThrottled {
            partition_id: expected_part
        }
    );

    // 3. Malformed XDR payload is routed to DLQ without crashing the stream
    let malformed = partitioner.dispatch(&ScpEnvelopeEvent {
        ledger_sequence: 50_000_124,
        validator_node_id: "GVALIDATOR1".to_string(),
        stage: ScpStage::Prepare,
        raw_xdr_base64: "BAD_XDR".to_string(),
    });
    assert!(matches!(
        malformed,
        PartitionDispatchOutcome::SentToDeadLetterQueue { .. }
    ));
    assert_eq!(partitioner.dlq_events.len(), 1);
    partitioner.ack_partition_delivery(expected_part);
}

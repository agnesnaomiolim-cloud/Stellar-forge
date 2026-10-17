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
use serde::{Deserialize, Serialize};

pub const DEFAULT_IPC_HANG_TIMEOUT_MS: u64 = 15_000;
pub const DEFAULT_SIGKILL_GRACE_MS: u64 = 5_000;
pub const DEFAULT_CORE_LOCK_PATH: &str = "/var/lib/stellar/core.lock";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IpcHealthState {
    Responsive,
    Hung,
    Disconnected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IpcProbeSnapshot {
    pub last_pong_timestamp_ms: u64,
    pub now_timestamp_ms: u64,
    pub pipe_connected: bool,
    pub latest_ingested_ledger: u64,
}

pub fn evaluate_ipc_health(snapshot: &IpcProbeSnapshot, hang_timeout_ms: u64) -> IpcHealthState {
    if !snapshot.pipe_connected {
        return IpcHealthState::Disconnected;
    }
    let elapsed = snapshot
        .now_timestamp_ms
        .saturating_sub(snapshot.last_pong_timestamp_ms);
    if elapsed > hang_timeout_ms {
        IpcHealthState::Hung
    } else {
        IpcHealthState::Responsive
    }
}

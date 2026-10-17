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
use crate::captive::ipc::{
    evaluate_ipc_health, IpcHealthState, IpcProbeSnapshot, DEFAULT_CORE_LOCK_PATH,
    DEFAULT_IPC_HANG_TIMEOUT_MS, DEFAULT_SIGKILL_GRACE_MS,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProcessTerminationSignal {
    None,
    SigTerm,
    SigKill,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupervisorRecoveryAction {
    pub ipc_state: IpcHealthState,
    pub signal_sent: ProcessTerminationSignal,
    pub stale_lock_removed: bool,
    pub restart_scheduled: bool,
    pub resume_from_ledger: u64,
}

pub struct CaptiveCoreSupervisor {
    pub hang_timeout_ms: u64,
    pub sigkill_grace_ms: u64,
    pub lockfile_path: String,
    pub total_restarts: u64,
}

impl Default for CaptiveCoreSupervisor {
    fn default() -> Self {
        Self {
            hang_timeout_ms: DEFAULT_IPC_HANG_TIMEOUT_MS,
            sigkill_grace_ms: DEFAULT_SIGKILL_GRACE_MS,
            lockfile_path: DEFAULT_CORE_LOCK_PATH.to_string(),
            total_restarts: 0,
        }
    }
}

impl CaptiveCoreSupervisor {
    pub fn evaluate_and_recover(
        &mut self,
        probe: &IpcProbeSnapshot,
        sigterm_sent_at_ms: Option<u64>,
        lockfile_exists: bool,
    ) -> SupervisorRecoveryAction {
        let ipc_state = evaluate_ipc_health(probe, self.hang_timeout_ms);

        match ipc_state {
            IpcHealthState::Responsive => SupervisorRecoveryAction {
                ipc_state,
                signal_sent: ProcessTerminationSignal::None,
                stale_lock_removed: false,
                restart_scheduled: false,
                resume_from_ledger: probe.latest_ingested_ledger,
            },
            IpcHealthState::Hung => {
                if let Some(sent_at) = sigterm_sent_at_ms {
                    if probe.now_timestamp_ms.saturating_sub(sent_at) >= self.sigkill_grace_ms {
                        self.total_restarts += 1;
                        return SupervisorRecoveryAction {
                            ipc_state,
                            signal_sent: ProcessTerminationSignal::SigKill,
                            stale_lock_removed: lockfile_exists,
                            restart_scheduled: true,
                            resume_from_ledger: probe.latest_ingested_ledger,
                        };
                    }
                }
                SupervisorRecoveryAction {
                    ipc_state,
                    signal_sent: ProcessTerminationSignal::SigTerm,
                    stale_lock_removed: false,
                    restart_scheduled: false,
                    resume_from_ledger: probe.latest_ingested_ledger,
                }
            }
            IpcHealthState::Disconnected => {
                self.total_restarts += 1;
                SupervisorRecoveryAction {
                    ipc_state,
                    signal_sent: ProcessTerminationSignal::None,
                    stale_lock_removed: lockfile_exists,
                    restart_scheduled: true,
                    resume_from_ledger: probe.latest_ingested_ledger,
                }
            }
        }
    }

    pub fn cleanup_stale_lockfile_on_disk(&self, path: &Path) -> std::io::Result<bool> {
        if path.exists() {
            std::fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

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
#[path = "../src/captive/mod.rs"]
mod captive;

use captive::ipc::{IpcHealthState, IpcProbeSnapshot};
use captive::supervisor::{CaptiveCoreSupervisor, ProcessTerminationSignal};

#[test]
fn test_captive_supervisor_hang_escalation_and_crash_lockfile_recovery() {
    let mut supervisor = CaptiveCoreSupervisor::default();
    assert_eq!(
        supervisor.lockfile_path,
        captive::ipc::DEFAULT_CORE_LOCK_PATH
    );

    // 1. Responsive IPC (< 15s) -> no action
    let ok_probe = IpcProbeSnapshot {
        last_pong_timestamp_ms: 10_000,
        now_timestamp_ms: 20_000,
        pipe_connected: true,
        latest_ingested_ledger: 500,
    };
    let res_ok = supervisor.evaluate_and_recover(&ok_probe, None, true);
    assert_eq!(res_ok.ipc_state, IpcHealthState::Responsive);
    assert_eq!(res_ok.signal_sent, ProcessTerminationSignal::None);
    assert!(!res_ok.restart_scheduled);

    // 2. Hung IPC (> 15s) -> sends SIGTERM first
    let hung_probe = IpcProbeSnapshot {
        last_pong_timestamp_ms: 10_000,
        now_timestamp_ms: 26_000,
        pipe_connected: true,
        latest_ingested_ledger: 500,
    };
    let res_sigterm = supervisor.evaluate_and_recover(&hung_probe, None, true);
    assert_eq!(res_sigterm.ipc_state, IpcHealthState::Hung);
    assert_eq!(res_sigterm.signal_sent, ProcessTerminationSignal::SigTerm);
    assert!(!res_sigterm.restart_scheduled);

    // 3. Still hung after 5s grace period -> escalates to SIGKILL, cleans lockfile, and schedules restart
    let hung_after_grace = IpcProbeSnapshot {
        last_pong_timestamp_ms: 10_000,
        now_timestamp_ms: 31_500,
        pipe_connected: true,
        latest_ingested_ledger: 500,
    };
    let res_sigkill = supervisor.evaluate_and_recover(&hung_after_grace, Some(26_000), true);
    assert_eq!(res_sigkill.signal_sent, ProcessTerminationSignal::SigKill);
    assert!(res_sigkill.stale_lock_removed);
    assert!(res_sigkill.restart_scheduled);
    assert_eq!(res_sigkill.resume_from_ledger, 500);

    // 4. Real temp lockfile removal verification
    let temp_dir = tempfile::tempdir().unwrap();
    let lock_path = temp_dir.path().join("core.lock");
    std::fs::write(&lock_path, b"locked").unwrap();
    assert!(supervisor
        .cleanup_stale_lockfile_on_disk(&lock_path)
        .unwrap());
    assert!(!lock_path.exists());
}

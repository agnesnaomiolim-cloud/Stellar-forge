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
//! Health probing for the Captive Core IPC endpoint.

use std::time::Duration;

use reqwest::Client;

/// Probe a Captive Core/RPC health endpoint with a bounded timeout.
pub async fn probe_http(client: &Client, endpoint: &str, timeout: Duration) -> Result<(), String> {
    client
        .get(endpoint)
        .timeout(timeout)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

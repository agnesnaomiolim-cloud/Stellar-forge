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
use stellar_k8s::backup::SecretRotationConfig;

#[tokio::test]
async fn test_secret_rotation_config_default() {
    let config = SecretRotationConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.schedule, "0 0 1 * *");
    assert_eq!(config.password_length, 32);
    assert_eq!(config.db_timeout_seconds, 30);
    assert_eq!(config.max_retries, 3);
    assert!(!config.audit_logging_enabled);
    assert!(config.audit_log_destination.is_none());
    assert!(config.notification_webhook.is_none());
}

#[tokio::test]
async fn test_secret_rotation_config_round_trip() {
    let config = SecretRotationConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    let deserialized: SecretRotationConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(config, deserialized);
}
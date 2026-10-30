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
// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! CRD Validation Helper Functions
//!
//! Provides validation for Kubernetes custom resource name strings following
//! Kubernetes RFC 1123 DNS label conventions.

/// Maximum allowed length for a Kubernetes DNS-1123 resource name label (RFC 1123).
pub const MAX_LABEL_LENGTH: usize = 63;

/// Validates a custom resource name string against Kubernetes DNS-1123 label rules.
///
/// Rules:
/// - Must not be empty (minLength: 1)
/// - Must not exceed 63 characters (maxLength: 63)
/// - Must contain only lowercase alphanumeric characters and hyphens ('-')
/// - Must start and end with an alphanumeric character (no leading or trailing hyphens)
pub fn validate_resource_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("resource name must not be empty (minLength: 1)".to_string());
    }
    if name.len() > MAX_LABEL_LENGTH {
        return Err(format!(
            "resource name '{name}' exceeds {MAX_LABEL_LENGTH} characters (maxLength: {MAX_LABEL_LENGTH})"
        ));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(format!(
            "resource name '{name}' is invalid: must match ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ (lowercase alphanumeric and hyphens only, no leading/trailing hyphens)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_kubernetes_resource_names() {
        // Representative valid Kubernetes resource names that must pass validation
        let valid_names = [
            "stellar-node",
            "my-private-net",
            "node-1",
            "validator-core-0",
            "a",
            "1",
            "horizon-rpc-node",
            "stellar-vault-testnet-123",
            "test-net-42",
        ];

        for name in valid_names {
            assert!(
                validate_resource_name(name).is_ok(),
                "Expected valid resource name '{name}' to pass validation"
            );
        }

        // Boundary case: exactly 63 characters should pass
        let max_len_name = "a".repeat(MAX_LABEL_LENGTH);
        assert!(
            validate_resource_name(&max_len_name).is_ok(),
            "Expected 63-character resource name to pass"
        );

        // Representative invalid Kubernetes resource names that must fail validation:
        // 1. Empty string
        let empty_res = validate_resource_name("");
        assert!(empty_res.is_err(), "Expected empty string to fail validation");
        assert!(empty_res.unwrap_err().contains("empty"));

        // 2. Exceeds max length (64 characters)
        let too_long = "a".repeat(MAX_LABEL_LENGTH + 1);
        let too_long_res = validate_resource_name(&too_long);
        assert!(too_long_res.is_err(), "Expected 64-character name to fail validation");
        assert!(too_long_res.unwrap_err().contains("exceeds 63 characters"));

        // 3. Uppercase letters (DNS-1123 requires lowercase)
        let uppercase_names = ["StellarNode", "My-Net", "MYNET", "nodeA", "STELLAR"];
        for bad in uppercase_names {
            let res = validate_resource_name(bad);
            assert!(res.is_err(), "Expected uppercase name '{bad}' to fail validation");
            assert!(res.unwrap_err().contains("invalid"));
        }

        // 4. Leading or trailing hyphens
        let bad_hyphen_names = ["-leading-hyphen", "trailing-hyphen-", "-both-ends-", "-"];
        for bad in bad_hyphen_names {
            assert!(
                validate_resource_name(bad).is_err(),
                "Expected leading/trailing hyphen name '{bad}' to fail validation"
            );
        }

        // 5. Special characters, underscores, and spaces
        let special_char_names = [
            "node_name",      // underscore is invalid in DNS-1123 label
            "node@name",      // at-sign
            "node.name",      // dot is invalid in DNS-1123 label
            "node!123",       // exclamation mark
            "node name",      // whitespace
            "; drop table",   // command/SQL injection
            "node#1",         // hash
            "node$core",      // dollar sign
            "node/core",      // slash
            "node:port",      // colon
        ];
        for bad in special_char_names {
            assert!(
                validate_resource_name(bad).is_err(),
                "Expected special character name '{bad}' to fail validation"
            );
        }
    }
}
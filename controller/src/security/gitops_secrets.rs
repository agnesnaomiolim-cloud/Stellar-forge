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
//! Legacy GitOps seed adapter. Direct SOPS/SealedSecret decryption is unsupported.
//!
//! This unregistered module previously contained a base64-encoded, malformed
//! prototype rather than compilable Rust. It must not be used as a cryptographic
//! implementation. All legacy entry points fail closed without reading seed
//! values, environment credentials or key-provider callbacks.
//!
//! Configure the active operator through
//! `StellarNode.spec.validatorConfig.seedSecretSource`: `externalRef` delegates
//! synchronization to External Secrets Operator; `csiRef` mounts a provider file;
//! `vaultRef` configures Vault Agent injection. See `src/controller/kms_secret.rs`
//! and `docs/security/rbac-hardening.md` for the supported interfaces.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// Explicit failure for a legacy interface that never provided working crypto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOpsSecretsError {
    UnsupportedLegacyDecryption,
}

impl fmt::Display for GitOpsSecretsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "direct GitOps secret decryption is unsupported; configure seedSecretSource.externalRef, csiRef or vaultRef",
        )
    }
}

impl std::error::Error for GitOpsSecretsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOpsSecretKind {
    Sops,
    SealedSecret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedSecretEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedSecret {
    pub name: String,
    pub namespace: Option<String>,
    pub kind: GitOpsSecretKind,
    pub entries: Vec<DecryptedSecretEntry>,
}

pub trait AgeIdentityProvider: Send + Sync {
    fn identity(&self) -> Result<String, GitOpsSecretsError>;
}

/// Retained only for source compatibility; it cannot authorize decryption.
pub struct InMemoryAgeIdentity;

impl InMemoryAgeIdentity {
    pub fn new(_identity: String) -> Self {
        Self
    }
}

impl AgeIdentityProvider for InMemoryAgeIdentity {
    fn identity(&self) -> Result<String, GitOpsSecretsError> {
        Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
    }
}

pub struct EnvAgeIdentity;

impl EnvAgeIdentity {
    pub fn new(_var_name: impl Into<String>) -> Self {
        Self
    }
}

impl AgeIdentityProvider for EnvAgeIdentity {
    fn identity(&self) -> Result<String, GitOpsSecretsError> {
        Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
    }
}

pub trait SealedSecretKeyProvider: Send + Sync {
    fn decryption_key(&self, name: &str) -> Result<Vec<u8>, GitOpsSecretsError>;
}

#[derive(Default)]
pub struct InMemorySealedSecretKeyProvider;

impl InMemorySealedSecretKeyProvider {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(&mut self, _name: impl Into<String>, _key: Vec<u8>) {}
}

impl SealedSecretKeyProvider for InMemorySealedSecretKeyProvider {
    fn decryption_key(&self, _name: &str) -> Result<Vec<u8>, GitOpsSecretsError> {
        Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
    }
}

pub fn parse_gitops_manifest(
    _path: &PathBuf,
    _raw: &[u8],
) -> Result<GitOpsSecretKind, GitOpsSecretsError> {
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub fn decrypt_sops_manifest(
    _path: &PathBuf,
    _raw: &[u8],
    _identity: &dyn AgeIdentityProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub fn decrypt_sealed_secret_manifest(
    _path: &PathBuf,
    _raw: &[u8],
    _key_provider: &dyn SealedSecretKeyProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub fn decrypt_gitops_manifest(
    _path: &PathBuf,
    _raw: &[u8],
    _age_identity: &dyn AgeIdentityProvider,
    _sealed_key_provider: &dyn SealedSecretKeyProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub fn load_gitops_secret_from_disk(
    _path: &PathBuf,
    _age_identity: &dyn AgeIdentityProvider,
    _sealed_key_provider: &dyn SealedSecretKeyProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub fn verify_decrypted_payload_integrity(
    _path: &PathBuf,
    _decrypted: &[u8],
    _expected_digest: &str,
) -> Result<(), GitOpsSecretsError> {
    // Never claim integrity verification succeeded for an unsupported payload.
    Err(GitOpsSecretsError::UnsupportedLegacyDecryption)
}

pub type GitOpsSecretResolver = Arc<dyn AgeIdentityProvider>;

#[cfg(test)]
mod tests {
    use super::*;

    struct UnreadableProvider;
    impl AgeIdentityProvider for UnreadableProvider {
        fn identity(&self) -> Result<String, GitOpsSecretsError> {
            panic!("legacy adapter must not access identities")
        }
    }
    impl SealedSecretKeyProvider for UnreadableProvider {
        fn decryption_key(&self, _name: &str) -> Result<Vec<u8>, GitOpsSecretsError> {
            panic!("legacy adapter must not access keys")
        }
    }

    #[test]
    fn legacy_entry_points_fail_without_accessing_secret_sources() {
        let path = PathBuf::from("nonexistent-secret-file");
        let provider = UnreadableProvider;
        let expected = GitOpsSecretsError::UnsupportedLegacyDecryption;
        assert_eq!(parse_gitops_manifest(&path, b""), Err(expected.clone()));
        assert_eq!(
            decrypt_sops_manifest(&path, b"", &provider),
            Err(expected.clone())
        );
        assert_eq!(
            decrypt_sealed_secret_manifest(&path, b"", &provider),
            Err(expected.clone())
        );
        assert_eq!(
            decrypt_gitops_manifest(&path, b"", &provider, &provider),
            Err(expected.clone())
        );
        assert_eq!(
            load_gitops_secret_from_disk(&path, &provider, &provider),
            Err(expected.clone())
        );
        assert_eq!(
            verify_decrypted_payload_integrity(&path, b"", ""),
            Err(expected)
        );
    }
}

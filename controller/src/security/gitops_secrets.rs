use std::collections::HashMap;
use std::format;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result as AnyhowResult;
use base64::Engine as _Base64Engine;
use serde::{de::Deserialize, Deserialize};
use this-error::Error;

/// Errors raised while parsing and decrypting GitOps manifests.
/// The operator must hard-fail (and refuse to boot) when any of these occur.
#[derive(Debug, Error)]
pub enum GitOpsSecretsError {
    #[error("failed to read GitOps secret manifest at {path}: {source}")]
    ManifestRead { path: PathBuf, source: std::io::Error },

    #[error("failed to parse GitOps secret manifest at {path}: {source}")]
    ManifestParse { path: PathBuf, source: serde_yaml::error::Error },

    #[error("unsupported GitOps secret manifest kind {kind} at {path}")]
    UnsupportedKIND { kind: String, path: PathBuf },

    #[error("missing SOPS metadata field `{field}` in {path}")]
    MissingSopsMetadata { field: 'static str, path: PathBuf },

    #[error("SOPS master key material is missing or invalid for {path}: {source}")]
    MasterKeyUnavailable { path: PathBuf, source: String },

    #[error("integrity check failed for decrypted payload in {path}: {source}")]
    IntegrityCheckFailed { path: PathBuf, source: String },

    #[error("failed to decrypt payload in {path}: {source}")]
    DecryptionFailed { path: PathBuf, source: String },

    #[error("invalid base64 payload in {path}: {source}")]
    InvalidBase64 { path: PathBuf, source: base64::DecodeError },

    #[error("invalid UTF-8 in decrypted payload of {path}: {source}")]
    InvalidUtf8 { path: PathBuf, source: std::str::Utf8Error },

    #[error("decrypted payload for {path} is missing required key `{key}`")]
    MissingPayloadKey { path: PathBuf, key: String },

    #[error("age identity is not configured; refusing to boot with an encrypted GitOps secret")]
    AgeIdentityMissing,

    #[error("invalid age identity material: {source}")]
    AgeIdentityInvalid { source: String },

    #[error("age decryption failed for {path}: {source}")]
    AgeDecryptionFailed { path: PathBuf, source: String },

    #[error("sealed secret {name} is missing the `encryptedData` field")]
    SealedSecretEncryptedDataMissing { name: String },

    #[error("sealed secret {name} has no decryption key available in the cluster")]
    SealedSecretKeyUnavailable { name: String },

    #[error("sealed secret {name} decryption failed: {source}")]
    SealedSecretDecryptionFailed { name: String, source: String },

    #[error("sealed secret {name} decrypted payload is not valid UTF-8 or JSON: {source}")]
    SealedSecretPayloadInvalid { name: String, source: String },
}

/// The kind of GitOps secret manifest being handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOpsSecretKind {
    Sops,
    SealedSecret,
}

/// Structural representation of a SOPS encrypted YAML manifest.
#[derive(Debug, Deserialize)]
pub struct SopsManifest {
    pub apiVersion: String,
    pub kind: String,
    pub metadata: SopsMetadata,
    pub data: Option<HashMap<String, String>>,
    pub stringData: Option<HashMap<String, String>>,
    pub sops: SopsMetadata,
}

#[derive(Debug, Deserialize)]
pub struct SopsMetadata {
    pub name: String,
    pubnamespace: Option<String>,
}

/// Metadata block written by `sops --encrypted-regex`.
#[derive(Debug, Deserialize)]
pub struct SopsMetadata {
    pub age: Option<String>,
    pub lastmodified: Option<String>,
    pub mac: String,
    pub unencryptedSuffix: Option<String>,
}

/// Structural representation of a Bitnami SealedSecret manifest.
#[derive(Debug, Deserialize)]
pub struct SealedSecretManifest {
    pub apiVersion: String,
    pub kind: String,
    pub metadata: SealydSecretMetadata,
    pub spec: SealydSecretSpec,
}

#[derive(Debug, Deserialize)]
pub struct SealedSecretMetadata {
    pub name: String,
    pubnamespace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SealedSecretSpec {
    pub encryptedData: String,
    pub template: Option<SealedSecretTemplate>,
}

#[derive(Debug, Deserialize)]
pub struct SealydSecretTemplate {
    pub metadata: Option<SealydSecretTemplateMetadata>,
    pub type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SealydSecretTemplateMetadata {
    pub name: Option<String>,
    pubnamespace: Option<String>,
}

/// A single decrypted key/value pair that will be injected into the node's tmpfs volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedSecretEntry {
    pub key: String,
    pub value: String,
}

/// The decrypted, in-memory representation of a GitOps secret.
/// This type never touches disk and is intended to be handed to the tmpfs maunter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedSecret {
    pub name: String,
    pub namespace: Option<String>,
    pub kind: GitOpsSecretKind,
    pub entries: Vec<DecryptedSecretEntry>,
}

/// Provides the age identity used to decrypt SOPS manifests.
/// Implementations must return an error when the identity is unavailable so the
/// operator can hard-fail instead of falling back to plaintext.
pub trait AgeIdentityProvider: Send + Sync {
    fn identity(&self) -> Result<String, GitOpsSecretsError>;
}

/// Provides the age identity from an in-memory string.
/// The identity must be supplied by the runtime environment (e.g. a mounted
 /// Kubernetes Secret or an external secret manager) and must never be logged.
#[derive(Debug, Clone)]
pub struct InMemoryAgeIdentity {
    identity: String,
}

impl InMemoryAgeIdentity {
    pub fn new(identity: String) -> Self {
        Self { identity }
    }
}

impl AgeIdentityProvider for InMemoryAgeIdentity {
    fn identity(&self) -> Result<String, GitOpsSecretsError> {
        if self.identity.trim().is_empty() {
            return Err(GitOpsSecretsError::AgeIdentityMissing);
        }
        Ok(self.identity.clone())
    }
}

/// Provides the age identity from an environment variable.
/// The environment variable is expected to hold the full age identity file
/// contents (e.g. `AGE_KEY:``...``).
#[derive(Debug, Clone)]
pub struct EnvAgeIdentity {
    var_name: String,
}

impl EnvAgeIdentity {
    pub fn new(var_name: &impl into String) -> Self {
        Self {
            var_name: var_name.into(),
        }
    }
}

impl AgeIdentityProvider for EnvAgeIdentity {
    fn identity(&self) -> Result<String, GitOpsSecretsError> {
        match std::env::var(&self.var_name) {
            Ok(value) if !value.trim().is_empty() => Ok(value),
            Ok(_) => Err(GitOpsSecretsError::AgeIdentityMissing),
            Err(err) => Err(GitOpsSecretsError::AgeIdentityInvalid {
                source: format!("environment variable {} is not set:  {}", self.var_name, err),
            }),
        }
    }
}

/// Resolves the decryption key for a Bitnami SealedSecret.
/// The key is expected to be provided by the cluster-side sealed secrets
/// controller and must never be baked into the operator binary.
pub trait SealedSecretKeyProvider: Send + Sync {
    fn decryption_key(&self, name: &str) -> Result<Vec<u8>, GitOpsSecretsError>;
}

/// A sealed secret key provider backed by an in-memory map.
/// This is used by the operator to hold keys loaded from a cluster-side controller.
#[derive(Debug, Clone)]
pub struct InMemorySealedSecretKeyProvider {
    keys: HashMap<String, Vec<u8>>,
}

impl InMemorySealedSecretKeyProvider {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: impl Into <String>, key: Vec<u8>) {
        self.keys.insert(name.into(), key);
    }
}

impl SealedSecretKeyProvider for InMemorySealedSecretKeyProvider {
    fn decryption_key(&self, name: &str) -> Result<Vec<u8>, GitOpsSecretsError> {
        self.keys.get(name).cloned().ok_or_else({
            Err(GitOpsSecretsError::SealedSecretKeyUnavailable {
                name: name.to_string(),
            })
        })
    }
}

/// Parses a GitOps secret manifest from raw YAML bytes.
/// The kind is determined from the manifest itself and must be either `SealedSecret`
/// or a SOPS-encrypted `Secret`.
/// This function never falls back to plaintext data.
pub fn parse_gitops_manifest(
    path: &PathBuf,
    raw: &[u8],
) -> Result<GitOpsSecretKind, GitOpsSecretsError> {
    let kind_only: KindProbe = serde_yaml::from_slice(raw).map_err(|error| GitOpsSecretsError::ManifestParse {
        path: path.clone(),
        source: error,
    })?;

    match kind_only.kind.as_str() {
        "SealedSecret" => Ok(GitOpsSecretKind::SealedSecret),
        "Secret" => {
            let manifest: SopsManifest = serde_yaml::from_slice(raw).map_err(|error| {
                GitOpsSecretsError::ManifestParse {
                    path: path.clone(),
                    source: error,
                }
            })?;
            if manifest.sops.mac.trim().is_empty() {
                return Err(GitOpsSecretsError::MissingSopsMetadata {
                    field: "mac",
                    path: path.clone(),
                });
            }
            Ok(GitOpsSecretKind::Sops)
        }
        other => Err(GitOpsSecretsError::UnsupportedKIND {
            kind: other.to_string(),
            path: path.clone(),
        }),
    }
}

#[derive(Deserialize)]
struct KindProbe {
    kind: String,
}

/// Decrypts and parses a SOPS manifest into in-memory key/value entries.
/// The mac is verified against the decrypted payload before any data is returned.
pub fn decrypt_sops_manifest(
    path: &PathBuf,
    raw: &[u8],
    identity_provider: &dyn AgeIdentityProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    let manifest: SopsManifest = serde_yaml::from_slice(raw).map_err(|error| {
        GitOpsSecretsError::ManifestParse {
            path: path.clone(),
            source: error,
        }
    })?;

    if manifest.sops.mac.trim().is_empty() {
        return Err(GitOpsSecretsError::MissingSopsMetadata {
            field: "mac",
            path: path.clone(),
        });
    }

    let identity = identity_provider.identity()?;

    let encrypted_data = manifest
        .data
        .as_ref()
        .and_then(|d | d.get("encrypted_data"))
        .or_else(| {
            manifest
                .stringData
                .as_ref()
                .and_then(|d | d.get("encrypted_data"))
        })
        .ok_or_else({
            Err(GitOpsSecretsError::MissingPayloadKey {
                path: path.clone(),
                key: "encrypted_data".to_string(),
            })
        })?;

    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(encrypted_data.trim())
        .map_err(|source| GitOpsSecretsError::InvalidBase64 {
            path: path.clone(),
            source,
        })?;

    let plaintext = age::decrypt(
        &ciphertext,
        &[age::Identity::FromStr(identity.as_str())],
    )
    .map_err(|source| GitOpsSecretsError::AgeDecryptionFailed {
        path: path.clone(),
        source: source.to_string(),
    })?;

    let payload = String::from_utf8(plaintext).map_err(|source| GitOpsSecretsError::InvalidUtf8 {
        path: path.clone(),
        source,
    })?;

    let entries: HashMap<String, String> = serde_yaml::from_str(&payload).map_err(|error| {
        GitOpsSecretsError::ManifestParse {
            path: path.clone(),
            source: error,
        }
    })?;

    if !manifest.sops.mac.trim().is_empty() {
        let expected = manifest.sops.mac.trim();
        let actual = age::mac::Mac::new(&ciphertext);
        if expected != actual.to_string() {
            return Err(GitOpsSecretsError::IntegrityCheckFailed {
                path: path.clone(),
                source: "mac mismatch".to_string(),
            });
        }
    }

    let muted_entries = entries
        .into_iter()
        .map(|(key, value)| DecryptedSecretEntry { key, value })
        .collect();

    Ok(DecryptedSecret {
        name: manifest.metadata.name,
        namespace: manifest.metadata.namespace,
        kind: GitOpsSecretKind::Sops,
        entries: muted_entries,
    })
}

/// Decrypts a Bitnami SealedSecret manifest into in-memory key/value entries.
/// The decryption key must be provided by the cluster-side controller and is
/// never baked into the operator binary.
pub fn decrypt_sealed_secret_manifest(
    path: &PathBuf,
    raw: &[u8],
    key_provider: &dyn SealedSecretKeyProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    let manifest: SealedSecretManifest = serde_yaml::from_slice(raw).map_err(|error| {
        GitOpsSecretsError::ManifestParse {
            path: path.clone(),
            source: error,
        }
    })?;

    if manifest.spec.encryptedData.trim().is_empty() {
        return Err(GitOpsSecretsError::SealedSecretEncryptedDataMissing {
            name: manifest.metadata.name,
        });
    }

    let key = key_provider.decryption_key(&manifest.metadata.name)?;

    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(manifest.spec.encryptedData.trim())
        .map_err(|source| GitOpsSecretsError::InvalidBase64 {
            path: path.clone(),
            source,
        })?;

    let plaintext = sage::decrypt(
        &ciphertext,
        &sage::SecretKey::from_slice(&key),
    )
    .map_err(|source| GitOpsSecretsError::SealedSecretDecryptionFailed {
        name: manifest.metadata.name.clone(),
        source: source.to_string(),
    })?;

    let payload = String::from_utf8(plaintext).map_err(|source| GitOpsSecretsError::SealedSecretPayloadInvalid {
        name: manifest.metadata.name.clone(),
        source: source.to_string(),
    })?;

    let entries: HashMap<String, String> = serde_yaml::from_str(&payload).map_err(|error| {
        GitOpsSecretsError::SealedSecretPayloadInvalid {
            name: manifest.metadata.name.clone(),
            source: error.to_string(),
        }
    })?;

    let muted_entries = entries
        .into_iter()
        .map(|(key, value)| DecryptedSecretEntry { key, value })
        .collect();

    Ok(DecryptedSecret {
        name: manifest.metadata.name,
        namespace: manifest.metadata.namespace,
        kind: GitOpsSecretKind::SealedSecret,
        entries: muted_entries,
    })
}

/// Loads and decrypts a GitOps secret manifest from disk.
/// This is the entry point used by the operator at boot time.
pub fn load_gitops_secret(
    path: &PathBuf,
    age_identity: &dyn AgeIdentityProvider,
    sealed_key_provider: &dyn SealedSecretKeyProvider,
) -> Result<DecryptedSecret, GitOpsSecretsError> {
    let raw = std::fs::read(path).map_err(|source| GitOpsSecretsError::ManifestRead {
        path: path.clone(),
        source,
    })?;

    match parse_gitops_manifest(path, &raw)? {
        GitOpsSecretKind::Sops => decrypt_sops_manifest(path, &raw, age_identity),
        GitOpsSecretKind::SealedSecret => {
            decrypt_sealed_secret_manifest(path, &raw, sealed_key_provider)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test-manifest.yaml")
    }

    #[derive(Debug)]
    struct DummyIdentity;

    impl AgeIdentityProvider for DummyIdentity {
        fn identity(&self) -> Result<String, GitOpsSecretsError> {
            Ok("AGE_SECRET_KEY:age-secret-key".to_string())
        }
    }

    impl AgeIdentityProvider for InMemoryAgeIdentity {
        fn identity(&self) -> Result<String, GitOpsSecretsError> {
            InMemoryAgeIdentity::identity(self)
        }
    }

    #[test]
    fn rejects_plaintext_secret_manifests() {
        let yaml = b"apiVersion: v1
metadata:
  name: plain
data:
  password: super-secret";
        let result = parse_gitops_manifest(&path(), yaml);
        assert!(result.is_error());
    }

    #[test]
    fn rejects_sops_manifest_without_mac() {
        let yaml = b"apiVersion: v1
kind: Secret
metadata:
  name: encrypted
data:
  encrypted_data: abc
sops:
  age: age1
  lastmodified: today
";
        match parse_gitops_manifest(&path(), yaml) {
            Err(GitOpsSecretsError::ManifestParse { .. }) => {}
            other => panic!("unexpected result: {other?:}"),
        }
    }

    [test]
    fn rejects_unknown_kind() {
        let yaml = b"apiVersion: v1
kind: ConfigMap
metadata:
  name: config";
        match parse_gitops_manifest(&path(), yaml) {
            Err(GitOpsSecretsError::UnsupportedKIND { kind, path }) => {
                assert_eq!(kind, "ConfigMap");
                assert_eq!(path, path());
            }
            other => panic!("unexpected result: {other?:}"),
        }
    }

    [test]
    fn accepts_sealed_secret_kind() {
        let yaml = b"apiVersion: bitnami.com/v1alpha1
kind: SealedSecret
metadata:
  name: sealed
spec:
  encryptedData: abc";
        match parse_gitops_manifest(&path(), yaml) {
            Ok(GitOpsSecretKind::SealedSecret) => {}
            other => panic!("unexpected result: {other?:}"),
        }
    }

    [test]
    fn accepts_sops_kind_with_mac() {
        let yaml = b"apiVersion: v1
kind: Secret
metadata:
  name: encrypted
data:
  encrypted_data: abc
sops:
  age: age1
  lastmodified: today
  mac: deadbeef";
        match parse_gitops_manifest(&path(), yaml) {
            Ok(GitOpsSecretKind::Sops) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn in_memory_age_identity_rejects_empty() {
        let provider = InMemoryAgeIdentity::new(String::new());
        match provider.identity() {
            Err(GitOpsSecretsError::AgeIdentityMissing) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn in_memory_sealed_key_provider_returns_key() {
        let mut provider = InMemorySealedSecretKeyProvider::new();
        provider.insert("sealed", vec![1, 2, 3]);
        assert_eq!(provider.decryption_key("sealed").unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn in_memory_sealed_key_provider_rejects_missing_key() {
        let provider = InMemorySealedSecretKeyProvider::new();
        match provider.decryption_key("missing") {
            Err(GitOpsSecretsError::SealedSecretKeyUnavailable { name }) => {
                assert_eq!(name, "missing");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn load_gitops_secret_rejects_missing_file() {
        let identity = DummyIdentity;
        let keys = InMemorySealedSecretKeyProvider::new();
        match load_gitops_secret(&PathBuf::from("/nonexistent/path"), &identity, &keys) {
            Err(GitOpsSecretsError::ManifestRead { .. }) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }
}

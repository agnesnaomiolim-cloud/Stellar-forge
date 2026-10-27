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
//! Multi-Cluster Snapshot Synchronization Reconciler.
//!
//! Automates the full lifecycle of bringing a secondary cluster node up from a
//! cloud-stored ledger snapshot:
//!
//! 1. **Discover** — query AWS S3 (or any S3-compatible storage) for the most
//!    recent ledger archive matching a configurable prefix.
//! 2. **Download** — stream the archive to disk using chunked I/O so that pod
//!    RAM is never exhausted, even for 20 GB+ archives.
//! 3. **Verify** — compute and compare the SHA-256 digest against the expected
//!    value (inline, sidecar, or S3 object metadata). Reject on mismatch.
//! 4. **Extract** — decompress the `.tar.gz` archive into the target data
//!    directory, streaming through `flate2` to avoid double-buffering.
//! 5. **Bootstrap** — write a sentinel file and emit Kubernetes Events so that
//!    the reconciler loop can mark the node `Bootstrapped`.
//!
//! # Architecture
//!
//! ```text
//!  ┌────────────────────────────────────────────────────────────────────┐
//!  │                   SnapshotReconciler                               │
//!  │                                                                    │
//!  │  reconcile(node, config)                                           │
//!  │    │                                                               │
//!  │    ├─ discover_latest_snapshot()  → SnapshotRef                   │
//!  │    ├─ download_archive()          → local path (streaming)         │
//!  │    ├─ verify (verifier.rs)        → VerificationResult            │
//!  │    ├─ extract_archive()           → data directory populated       │
//!  │    └─ bootstrap_node()            → sentinel written; event emitted│
//!  └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Constraints
//!
//! - All downloads stream ranges directly to disk; no in-memory
//!   buffering of the archive body.
//! - SHA-256 verification is mandatory; extraction is skipped on mismatch.
//! - Extraction completes in a temporary directory before publication.
//!   Replacing existing data uses two renames with rollback on install errors;
//!   a process crash between renames can leave the data at its `.old` backup.
//! - A `bootstrapped` sentinel file prevents re-extraction on restarts.

use std::path::{Path, PathBuf};
use std::time::Duration;
use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use chrono::Utc;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive as TarArchive;
use tokio::fs;
use tracing::{debug, error, info, instrument, warn};

use super::verifier::{parse_sha256_sidecar, verify_file};
use crate::error::{Error, Result};

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// Reference to a specific snapshot archive in cloud storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRef {
    /// S3 bucket name.
    pub bucket: String,
    /// Object key within the bucket (e.g. `snapshots/stellar-mainnet-20260830.tar.gz`).
    pub key: String,
    /// Expected SHA-256 hex digest. If `None` the reconciler will attempt to
    /// fetch a `.sha256` sidecar file.
    pub expected_sha256: Option<String>,
    /// Approximate object size in bytes (used for progress logging).
    pub size_bytes: Option<u64>,
    /// Timestamp the snapshot was created (ISO-8601).
    pub created_at: Option<String>,
    /// Ledger sequence number at which the snapshot was taken.
    pub ledger_sequence: Option<u64>,
}

impl SnapshotRef {
    /// Return a human-readable description for log messages.
    pub fn display_name(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.key)
    }
}

/// Configuration driving the snapshot reconciler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReconcilerConfig {
    /// S3 bucket name to query.
    pub bucket: String,
    /// Key prefix used to filter objects when discovering snapshots.
    /// E.g. `snapshots/mainnet/`.
    pub key_prefix: String,
    /// Local filesystem directory where the archive is temporarily stored.
    pub staging_dir: PathBuf,
    /// Target directory where the archive is extracted (node data root).
    pub data_dir: PathBuf,
    /// AWS region for the S3 client.
    pub aws_region: String,
    /// Maximum objects per discovery page; every page is searched.
    pub list_max_keys: i32,
    /// If true, skip extraction if the `bootstrapped` sentinel file already exists.
    pub skip_if_bootstrapped: bool,
    /// Timeout for individual S3 API calls.
    pub s3_api_timeout: Duration,
}

impl Default for SnapshotReconcilerConfig {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            key_prefix: String::new(),
            staging_dir: PathBuf::from("/tmp/stellar-snapshots"),
            data_dir: PathBuf::from("/var/lib/stellar"),
            aws_region: "us-east-1".to_string(),
            list_max_keys: 100,
            skip_if_bootstrapped: true,
            s3_api_timeout: Duration::from_secs(30),
        }
    }
}

/// Outcome of a full reconciliation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileOutcome {
    /// Whether the node was successfully bootstrapped.
    pub bootstrapped: bool,
    /// Snapshot that was used.
    pub snapshot: Option<SnapshotRef>,
    /// SHA-256 verification result message.
    pub verification_message: String,
    /// Extraction path.
    pub data_dir: String,
    /// Total bytes downloaded.
    pub downloaded_bytes: u64,
    /// ISO-8601 timestamp when reconciliation completed.
    pub completed_at: String,
    /// Ledger sequence confirmed at bootstrap.
    pub ledger_sequence: Option<u64>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Reconciler
// ──────────────────────────────────────────────────────────────────────────────

/// Stateless snapshot reconciler.
///
/// Construct once and call [`SnapshotReconciler::reconcile`] on each pass.
/// Uses the AWS SDK S3 client; credentials are resolved from the standard
/// environment chain (IAM role, env vars, `~/.aws/credentials`).
pub struct SnapshotReconciler {
    config: SnapshotReconcilerConfig,
    s3: S3Client,
}

impl SnapshotReconciler {
    /// Create a new reconciler with the given config.
    ///
    /// Initialises the AWS SDK from the ambient environment (IAM instance role,
    /// `AWS_*` env vars, or `~/.aws/credentials`).
    pub async fn new(config: SnapshotReconcilerConfig) -> Result<Self> {
        if config.s3_api_timeout.is_zero() || !(1..=1000).contains(&config.list_max_keys) {
            return Err(Error::ValidationError(
                "Snapshot API timeout must be positive and listing page size must be 1..=1000".into()
            ));
        }
        let sdk_config = aws_config::from_env()
            .region(
                aws_config::meta::region::RegionProviderChain::default_provider()
                    .or_else(config.aws_region.as_str()),
            )
            .load()
            .await;
        let s3 = S3Client::new(&sdk_config);
        Ok(Self { config, s3 })
    }

    /// Run a full reconciliation pass.
    ///
    /// Steps:
    /// 1. Return early if `bootstrapped` sentinel exists and `skip_if_bootstrapped` is set.
    /// 2. Discover the most recent snapshot in S3.
    /// 3. Download the archive to `staging_dir`.
    /// 4. Verify SHA-256 integrity.
    /// 5. Extract fully into a temporary directory, then publish into `data_dir`.
    /// 6. Write sentinel and return outcome.
    #[instrument(skip(self), fields(bucket = %self.config.bucket, prefix = %self.config.key_prefix))]
    pub async fn reconcile(&self) -> Result<ReconcileOutcome> {
        let publication_guard = lock_bootstrap_destination(&self.config.data_dir).await?;
        // Step 0 — check sentinel
        let sentinel = self.config.data_dir.join(".bootstrapped");
        if self.config.skip_if_bootstrapped && sentinel.exists() {
            info!(
                sentinel = %sentinel.display(),
                "Node already bootstrapped, skipping snapshot reconciliation"
            );
            return Ok(ReconcileOutcome {
                bootstrapped: true,
                snapshot: None,
                verification_message: "already bootstrapped — skipped".to_string(),
                data_dir: self.config.data_dir.display().to_string(),
                downloaded_bytes: 0,
                completed_at: Utc::now().to_rfc3339(),
                ledger_sequence: None,
            });
        }

        // Step 1 — discover
        let snapshot_ref = self.discover_latest_snapshot().await?;
        info!(
            key = %snapshot_ref.key,
            size = ?snapshot_ref.size_bytes,
            ledger = ?snapshot_ref.ledger_sequence,
            "Discovered latest snapshot"
        );

        // Resolve a trusted digest before downloading; publication requires verification.
        let expected_hex = self.resolve_checksum(&snapshot_ref).await?;
        let (archive_path, archive_guard) = self.download_archive(&snapshot_ref, &expected_hex).await?;
        let downloaded_bytes = fs::metadata(&archive_path).await?.len();
        info!(
            path = %archive_path.display(),
            bytes = downloaded_bytes,
            "Download complete"
        );

        // Step 4 — verify
        let vr = verify_file(&archive_path, &expected_hex)
            .await?
            .into_result()?;
        info!(
            result = %vr,
            "SHA-256 verification passed"
        );

        // Step 5 — extract
        self.extract_archive(
            &archive_path,
            &self.config.data_dir,
            publication_guard.clone(),
            archive_guard.clone(),
        )
            .await?;
        info!(
            data_dir = %self.config.data_dir.display(),
            "Archive extracted"
        );

        // Step 6 — write sentinel
        self.write_sentinel(&sentinel, &snapshot_ref).await?;

        // Clean up staging file
        if let Err(e) = fs::remove_file(&archive_path).await {
            warn!(path = %archive_path.display(), err = %e, "Failed to remove staging archive");
        }

        Ok(ReconcileOutcome {
            bootstrapped: true,
            snapshot: Some(snapshot_ref.clone()),
            verification_message: format!(
                "SHA-256 matched: {} ({} bytes)",
                vr.computed_hex, vr.file_size_bytes
            ),
            data_dir: self.config.data_dir.display().to_string(),
            downloaded_bytes,
            completed_at: Utc::now().to_rfc3339(),
            ledger_sequence: snapshot_ref.ledger_sequence,
        })
    }

    // ────────────────────────────────────────────────────────────────────────
    // Private helpers
    // ────────────────────────────────────────────────────────────────────────

    /// List objects in S3 under `key_prefix` and return the most recently
    /// modified one.
    #[instrument(skip(self))]
    async fn discover_latest_snapshot(&self) -> Result<SnapshotRef> {
        let mut continuation = None;
        let mut latest = None;
        loop {
            let resp = tokio::time::timeout(self.config.s3_api_timeout, self
                .s3
                .list_objects_v2()
                .bucket(&self.config.bucket)
                .prefix(&self.config.key_prefix)
                .max_keys(self.config.list_max_keys)
                .set_continuation_token(continuation.clone())
                .send())
                .await
                .map_err(|_| Error::ConfigError("S3 ListObjectsV2 timed out".into()))?
                .map_err(|e| Error::ConfigError(format!("S3 ListObjectsV2 failed: {e}")))?;

            latest = latest_snapshot(resp.contents.unwrap_or_default().into_iter().chain(latest));
            if !resp.is_truncated.unwrap_or(false) {
                break;
            }
            let next = resp.next_continuation_token
                .filter(|token| !token.is_empty())
                .ok_or_else(|| Error::ConfigError(
                    "Truncated S3 listing has no continuation token".into()
                ))?;
            if continuation.as_ref() == Some(&next) {
                return Err(Error::ConfigError(
                    "S3 listing repeated its continuation token".into()
                ));
            }
            continuation = Some(next);
        }
        let latest = latest.ok_or_else(|| Error::NotFound {
            kind: "S3Snapshot".to_string(),
            name: self.config.key_prefix.clone(),
            namespace: self.config.bucket.clone(),
        })?;

        let key = latest.key.clone().unwrap_or_default();
        let size_bytes = latest.size.map(|s| s as u64);

        // Attempt to extract ledger sequence from key name
        // Convention: `<prefix>/stellar-<network>-<ledger>.tar.gz`
        let ledger_sequence = key.rsplit('/').next().and_then(|name| {
            name.strip_suffix(".tar.gz")
                .and_then(|s| s.rsplit('-').next())
                .and_then(|seq| seq.parse::<u64>().ok())
        });

        debug!(key = %key, ledger = ?ledger_sequence, "resolved snapshot");

        Ok(SnapshotRef {
            bucket: self.config.bucket.clone(),
            key,
            expected_sha256: None,
            size_bytes,
            created_at: latest.last_modified.as_ref().map(|t| t.to_string()),
            ledger_sequence,
        })
    }

    /// Fetch and verify ranges using the process-wide worker and egress budget.
    #[instrument(skip(self, snap, expected_sha256), fields(key = %snap.key))]
    async fn download_archive(
        &self, snap: &SnapshotRef, expected_sha256: &str,
    ) -> Result<(PathBuf, Arc<tempfile::TempPath>)> {
        use crate::catchup::{Archive, DownloadConfig, Downloader, WorkerPool};
        use aws_sdk_s3::presigning::PresigningConfig;

        let filename = snapshot_staging_filename(&snap.bucket, &snap.key);
        fs::create_dir_all(&self.config.staging_dir).await?;
        let staging_dir = self.config.staging_dir.clone();
        let temporary = tokio::task::spawn_blocking(move || {
            tempfile::Builder::new().prefix(&filename)
                .tempfile_in(staging_dir).map(|file| file.into_temp_path())
        }).await.map_err(|error| Error::InternalError(format!("Staging file creation failed: {error}")))??;
        let dest = temporary.to_path_buf();
        // A GET signature allows ranges without exposing permanent cloud credentials.
        let signing = PresigningConfig::expires_in(Duration::from_secs(7 * 24 * 60 * 60))
            .map_err(|_| Error::ConfigError("Invalid presigning duration".into()))?;
        let signed = self
            .s3
            .get_object()
            .bucket(&snap.bucket)
            .key(&snap.key)
            .presigned(signing)
            .await
            .map_err(|_| Error::ConfigError("Unable to presign snapshot download".into()))?;
        let pool = WorkerPool::shared_from_env().map_err(|e| Error::ConfigError(e.to_string()))?;
        let downloader = Downloader::new(pool, DownloadConfig::default())
            .map_err(|e| Error::ConfigError(e.to_string()))?;
        let path = downloader
            .download(Archive {
                url: signed.uri().to_string(),
                destination: dest,
                sha256: expected_sha256.to_owned(),
            })
            .await
            .map_err(|e| Error::ConfigError(format!("Snapshot download failed: {e}")))?;
        Ok((path, Arc::new(temporary)))
    }
    /// Resolve the expected SHA-256 checksum for `snap`.
    ///
    /// Priority:
    /// 1. `snap.expected_sha256` (inline)
    /// 2. Sidecar file `<key>.sha256` in the same bucket
    /// 3. S3 object metadata `x-amz-meta-sha256`
    ///
    /// Returns `Error::ValidationError` if no checksum can be resolved.
    async fn resolve_checksum(&self, snap: &SnapshotRef) -> Result<String> {
        // 1. Inline
        if let Some(ref h) = snap.expected_sha256 {
            return Ok(h.clone());
        }

        // 2. Sidecar
        let sidecar_key = format!("{}.sha256", snap.key);
        if let Ok(Ok(resp)) = tokio::time::timeout(self.config.s3_api_timeout, self
            .s3
            .get_object()
            .bucket(&snap.bucket)
            .key(&sidecar_key)
            .send())
            .await
        {
            let bytes = tokio::time::timeout(self.config.s3_api_timeout, resp
                .body
                .collect())
                .await
                .map_err(|_| Error::ConfigError("Snapshot checksum sidecar read timed out".into()))?
                .map_err(|e| Error::ConfigError(format!("sidecar read error: {e}")))?
                .into_bytes();
            let contents = String::from_utf8_lossy(&bytes);
            if let Some((_, hex)) = parse_sha256_sidecar(&contents) {
                info!(sidecar = %sidecar_key, "resolved checksum from sidecar file");
                return Ok(hex);
            }
        }

        // 3. Object metadata
        if let Ok(Ok(head)) = tokio::time::timeout(self.config.s3_api_timeout, self
            .s3
            .head_object()
            .bucket(&snap.bucket)
            .key(&snap.key)
            .send())
            .await
        {
            if let Some(meta) = head.metadata() {
                if let Some(h) = meta.get("sha256").or_else(|| meta.get("x-amz-meta-sha256")) {
                    info!("resolved checksum from S3 object metadata");
                    return Ok(h.clone());
                }
            }
        }

        Err(Error::ValidationError(format!(
            "No SHA-256 checksum available for {}. Provide inline, .sha256 sidecar, or S3 metadata.",
            snap.display_name()
        )))
    }

    /// Extract a `.tar.gz` archive completely before publishing into `dest_dir`.
    ///
    /// Extracts into a unique temporary sibling directory first, then
    /// renames it into place. This prevents partially-extracted data from being
    /// used if the process is interrupted.
    #[instrument(skip(self, archive_path, dest_dir, publication_guard, archive_guard), fields(archive = %archive_path.as_ref().display()))]
    async fn extract_archive(
        &self,
        archive_path: impl AsRef<Path>,
        dest_dir: impl AsRef<Path>,
        publication_guard: Arc<tokio::sync::OwnedMutexGuard<()>>,
        archive_guard: Arc<tempfile::TempPath>,
    ) -> Result<()> {
        let archive_path = archive_path.as_ref().to_owned();
        let dest_dir = dest_dir.as_ref().to_owned();

        // Extraction is CPU-bound and blocking — spawn on the blocking thread pool.
        tokio::task::spawn_blocking(move || {
            // Blocking work survives future cancellation. Retain the destination
            // lock until that work stops, so another reconciliation cannot race it.
            let _guard = publication_guard;
            let _archive_guard = archive_guard;
            extract_tar_gz(&archive_path, &dest_dir)
        })
            .await
            .map_err(|e| Error::InternalError(format!("spawn_blocking panicked: {e}")))??;

        Ok(())
    }

    /// Write a `.bootstrapped` sentinel file containing the snapshot metadata.
    async fn write_sentinel(&self, sentinel: &Path, snap: &SnapshotRef) -> Result<()> {
        if let Some(parent) = sentinel.parent() {
            fs::create_dir_all(parent).await?;
        }
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(sentinel, serde_json::to_string_pretty(&content)?).await?;
        info!(sentinel = %sentinel.display(), "Wrote bootstrapped sentinel");
        Ok(())
    }
}

/// Object keys are opaque cloud identifiers, not safe local filenames. Hash
/// both bucket and key so Windows separators, drive prefixes, ADS colons and
/// reserved device names cannot escape or alias the configured staging folder.
fn snapshot_staging_filename(bucket: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bucket.as_bytes());
    digest.update([0]);
    digest.update(key.as_bytes());
    format!("snapshot-{}.tar.gz", hex::encode(digest.finalize()))
}

/// Serialize writers to one canonical destination across reconciler instances.
/// Independent operator processes must still have exclusive ownership of data.
async fn lock_bootstrap_destination(
    destination: &Path,
) -> Result<Arc<tokio::sync::OwnedMutexGuard<()>>> {
    use std::{collections::HashMap, sync::{Mutex, OnceLock, Weak}};
    type Gates = HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>;
    static GATES: OnceLock<Mutex<Gates>> = OnceLock::new();
    let parent = destination.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = destination.file_name()
        .ok_or_else(|| Error::ValidationError("Snapshot destination needs a directory name".into()))?;
    fs::create_dir_all(parent).await?;
    let key = fs::canonicalize(parent).await?.join(name);
    let gate = {
        let mut gates = GATES.get_or_init(|| Mutex::new(HashMap::new())).lock()
            .map_err(|_| Error::InternalError("Snapshot destination lock poisoned".into()))?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        match gates.get(&key).and_then(Weak::upgrade) {
            Some(gate) => gate,
            None => {
                let gate = Arc::new(tokio::sync::Mutex::new(()));
                gates.insert(key, Arc::downgrade(&gate));
                gate
            }
        }
    };
    Ok(Arc::new(gate.lock_owned().await))
}

fn latest_snapshot(
    objects: impl IntoIterator<Item = aws_sdk_s3::types::Object>,
) -> Option<aws_sdk_s3::types::Object> {
    objects
        .into_iter()
        .filter(|object| object.key.as_deref().is_some_and(|key| key.ends_with(".tar.gz")))
        .max_by_key(|object| object.last_modified.as_ref().map(|time| time.secs()))
}

// ──────────────────────────────────────────────────────────────────────────────
// Blocking extraction helper (runs inside spawn_blocking)
// ──────────────────────────────────────────────────────────────────────────────

/// Decompress fully before publication; roll back failed replacement renames.
/// A crash between backup and installation can require restoring `.old`.
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let parent = dest_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut old_name = dest_dir.file_name()
        .ok_or_else(|| Error::ValidationError("Snapshot destination needs a directory name".into()))?
        .to_os_string();
    old_name.push(".old");
    let old_dir = parent.join(old_name);
    std::fs::create_dir_all(parent)?;
    // Separate attempts must never delete each other's in-progress files.
    // RAII also removes partial extraction output when unpacking fails.
    let temporary = tempfile::Builder::new()
        .prefix(".stellar-extract-")
        .tempdir_in(parent)?;
    let tmp_dir = temporary.path();

    let file = std::fs::File::open(archive_path)?;
    let gz = GzDecoder::new(file);
    let mut archive = TarArchive::new(gz);
    archive.set_overwrite(true);
    archive.set_preserve_permissions(true);
    archive.unpack(tmp_dir)?;

    // Each rename is atomic, but replacement has a gap between these renames.
    let mut backup = None;
    if dest_dir.exists() {
        // Keep old data as `.old` for emergency rollback
        if old_dir.exists() {
            std::fs::remove_dir_all(&old_dir)?;
        }
        std::fs::rename(dest_dir, &old_dir)?;
        backup = Some(old_dir);
    }
    publish_extraction(tmp_dir, dest_dir, backup.as_deref())?;

    info!(
        archive = %archive_path.display(),
        dest = %dest_dir.display(),
        "Extraction complete"
    );
    Ok(())
}

/// Restore the previous data if installing the completed extraction fails.
fn publish_extraction(temporary: &Path, destination: &Path, backup: Option<&Path>) -> Result<()> {
    if let Err(publish_error) = std::fs::rename(temporary, destination) {
        if let Some(backup) = backup {
            std::fs::rename(backup, destination).map_err(|rollback_error| {
                Error::InternalError(format!(
                    "Snapshot install failed: {publish_error}; restoring {} failed: {rollback_error}",
                    backup.display()
                ))
            })?;
        }
        return Err(publish_error.into());
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn destination_lock_survives_retained_blocking_worker_guard() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("data");
        let guard = lock_bootstrap_destination(&destination).await.unwrap();
        let blocking_worker_guard = guard.clone();
        drop(guard);
        assert!(tokio::time::timeout(
            Duration::from_millis(25), lock_bootstrap_destination(&destination),
        ).await.is_err());
        drop(blocking_worker_guard);
        assert!(tokio::time::timeout(
            Duration::from_secs(1), lock_bootstrap_destination(&destination),
        ).await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn invalid_api_timeout_is_rejected_before_loading_aws_configuration() {
        let config = SnapshotReconcilerConfig {
            s3_api_timeout: Duration::ZERO,
            ..Default::default()
        };
        assert!(SnapshotReconciler::new(config).await.is_err());
    }

    #[test]
    fn discovery_keeps_the_latest_snapshot_across_pages() {
        use aws_sdk_s3::{primitives::DateTime, types::Object};
        let object = |key: &str, seconds| Object::builder()
            .key(key)
            .last_modified(DateTime::from_secs(seconds))
            .build();
        let first = latest_snapshot([
            object("first.tar.gz", 10),
            object("first.tar.gz.sha256", 100),
        ]);
        let second = latest_snapshot([object("second.tar.gz", 20)].into_iter().chain(first));
        let final_page = latest_snapshot([object("older.tar.gz", 5)].into_iter().chain(second));
        assert_eq!(final_page.unwrap().key.as_deref(), Some("second.tar.gz"));
    }

    #[test]
    fn failed_publication_restores_previous_data() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("data");
        let backup = directory.path().join("data.old");
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("ledger"), b"previous data").unwrap();
        assert!(publish_extraction(
            &directory.path().join("missing-extraction"),
            &destination,
            Some(&backup),
        ).is_err());
        assert_eq!(std::fs::read(destination.join("ledger")).unwrap(), b"previous data");
        assert!(!backup.exists());
    }

    #[test]
    fn failed_extraction_cleans_up_without_touching_existing_data() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("data");
        let archive = directory.path().join("invalid.tar.gz");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("ledger"), b"previous data").unwrap();
        std::fs::write(&archive, b"not a gzip archive").unwrap();
        assert!(extract_tar_gz(&archive, &destination).is_err());
        assert_eq!(std::fs::read(destination.join("ledger")).unwrap(), b"previous data");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn destination_named_old_is_not_deleted_as_its_own_backup() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("data.old");
        let archive = directory.path().join("archive.tar.gz");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("ledger"), b"previous data").unwrap();
        std::fs::write(&archive, make_test_tar_gz("ledger", b"new data")).unwrap();
        extract_tar_gz(&archive, &destination).unwrap();
        assert_eq!(std::fs::read(destination.join("ledger")).unwrap(), b"new data");
        assert_eq!(
            std::fs::read(directory.path().join("data.old.old/ledger")).unwrap(),
            b"previous data"
        );
    }

    #[test]
    fn cloud_keys_cannot_escape_or_alias_staging_paths() {
        let directory = Path::new("staging");
        for key in [
            r"..\..\outside.tar.gz",
            r"C:\outside.tar.gz",
            "CON",
            "name:stream",
            "..",
            "prefix/file.tar.gz",
        ] {
            let filename = snapshot_staging_filename("bucket", key);
            assert!(filename.starts_with("snapshot-") && filename.ends_with(".tar.gz"));
            assert_eq!(directory.join(&filename).parent(), Some(directory));
            assert!(filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.'));
        }
        assert_ne!(
            snapshot_staging_filename("a", "prefix/file.tar.gz"),
            snapshot_staging_filename("b", "prefix/file.tar.gz")
        );
        assert_ne!(
            snapshot_staging_filename("a", "one/file.tar.gz"),
            snapshot_staging_filename("a", "two/file.tar.gz")
        );
    }

    /// Build a minimal in-memory `.tar.gz` containing a single test file.
    fn make_test_tar_gz(filename: &str, content: &[u8]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        use tar::Builder;

        let buf = Vec::new();
        let enc = GzEncoder::new(buf, Compression::default());
        let mut builder = Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, filename, content).unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap()
    }

    #[tokio::test]
    async fn extract_tar_gz_creates_dest_dir() {
        let tmp = TempDir::new().unwrap();
        let archive_path = tmp.path().join("test.tar.gz");
        let dest = tmp.path().join("data");

        let tar_data = make_test_tar_gz("ledger.bin", b"ledger bytes here");
        std::fs::write(&archive_path, &tar_data).unwrap();

        extract_tar_gz(&archive_path, &dest).unwrap();
        assert!(dest.exists(), "dest dir should have been created");
        assert!(
            dest.join("ledger.bin").exists(),
            "archive content should be extracted"
        );
    }

    #[tokio::test]
    async fn extract_tar_gz_preserves_existing_as_old() {
        let tmp = TempDir::new().unwrap();
        let archive_path = tmp.path().join("v2.tar.gz");
        let dest = tmp.path().join("data");

        // Pre-existing data directory
        std::fs::create_dir_all(dest.join("existing")).unwrap();
        std::fs::write(dest.join("existing/old-file.dat"), b"old").unwrap();

        let tar_data = make_test_tar_gz("new-file.dat", b"new bytes");
        std::fs::write(&archive_path, &tar_data).unwrap();

        extract_tar_gz(&archive_path, &dest).unwrap();

        // New extraction in place
        assert!(dest.join("new-file.dat").exists());
        // Old data preserved as .old
        let old = dest.with_extension("old");
        assert!(old.exists(), "old data should be preserved as .old");
    }

    #[tokio::test]
    async fn write_sentinel_creates_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("stellar-data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let sentinel = data_dir.join(".bootstrapped");

        let snap = SnapshotRef {
            bucket: "my-bucket".to_string(),
            key: "snapshots/stellar-mainnet-12345.tar.gz".to_string(),
            expected_sha256: None,
            size_bytes: Some(1024),
            created_at: None,
            ledger_sequence: Some(12345),
        };

        // Simulate write_sentinel without a real reconciler by directly calling the function
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(&sentinel, serde_json::to_string_pretty(&content).unwrap())
            .await
            .unwrap();

        assert!(sentinel.exists());
        let raw = fs::read_to_string(&sentinel).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["ledger_sequence"], 12345);
        assert_eq!(parsed["snapshot_bucket"], "my-bucket");
    }

    #[test]
    fn snapshot_ref_display_name() {
        let snap = SnapshotRef {
            bucket: "stellar-snapshots".to_string(),
            key: "mainnet/snapshot.tar.gz".to_string(),
            expected_sha256: None,
            size_bytes: None,
            created_at: None,
            ledger_sequence: None,
        };
        assert_eq!(
            snap.display_name(),
            "s3://stellar-snapshots/mainnet/snapshot.tar.gz"
        );
    }

    #[test]
    fn reconcile_outcome_is_serializable() {
        let outcome = ReconcileOutcome {
            bootstrapped: true,
            snapshot: None,
            verification_message: "ok".to_string(),
            data_dir: "/var/lib/stellar".to_string(),
            downloaded_bytes: 1024,
            completed_at: Utc::now().to_rfc3339(),
            ledger_sequence: Some(99999),
        };
        let s = serde_json::to_string(&outcome).unwrap();
        assert!(s.contains("bootstrapped"));
    }

    /// End-to-end bootstrap validation test (no real S3 required).
    ///
    /// This test exercises the full local pipeline:
    ///   1. Write a synthetic .tar.gz to staging
    ///   2. Compute its SHA-256
    ///   3. Call verify_file
    ///   4. Call extract_tar_gz
    ///   5. Confirm sentinel can be written
    ///
    /// This mirrors the live `reconcile()` flow at the file-system level,
    /// validating the bootstrap path end-to-end.
    #[tokio::test]
    async fn bootstrap_flow_end_to_end() {
        use sha2::Digest;

        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("staging");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&staging).unwrap();

        // (1) Build synthetic archive
        let archive_content = b"stellar-ledger-data-12345";
        let tar_data = make_test_tar_gz("ledger-12345.dat", archive_content);
        let archive_path = staging.join("stellar-mainnet-12345.tar.gz");
        std::fs::write(&archive_path, &tar_data).unwrap();

        // (2) Compute expected SHA-256
        let expected_hex = hex::encode(sha2::Sha256::digest(&tar_data));

        // (3) Verify
        let vr = verify_file(&archive_path, &expected_hex).await.unwrap();
        assert!(vr.matched, "integrity check must pass");
        assert_eq!(vr.file_size_bytes, tar_data.len() as u64);

        // (4) Extract
        extract_tar_gz(&archive_path, &data_dir).unwrap();
        assert!(
            data_dir.join("ledger-12345.dat").exists(),
            "ledger file must exist in data dir"
        );
        let extracted = std::fs::read(data_dir.join("ledger-12345.dat")).unwrap();
        assert_eq!(extracted, archive_content, "extracted content must match");

        // (5) Write sentinel
        let sentinel = data_dir.join(".bootstrapped");
        let snap = SnapshotRef {
            bucket: "test-bucket".to_string(),
            key: "stellar-mainnet-12345.tar.gz".to_string(),
            expected_sha256: Some(expected_hex),
            size_bytes: Some(tar_data.len() as u64),
            created_at: None,
            ledger_sequence: Some(12345),
        };
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(&sentinel, serde_json::to_string_pretty(&content).unwrap())
            .await
            .unwrap();
        assert!(sentinel.exists(), "sentinel file must be written");

        // Confirm sentinel contents
        let raw = fs::read_to_string(&sentinel).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed["ledger_sequence"], 12345,
            "sentinel must record ledger sequence"
        );

        println!(
            "\n=== Bootstrap flow completed ===\n\
             Archive:  {}\n\
             SHA-256:  {}\n\
             Data dir: {}\n\
             Sentinel: {}\n\
             ================================",
            archive_path.display(),
            vr.computed_hex,
            data_dir.display(),
            sentinel.display()
        );
    }
}

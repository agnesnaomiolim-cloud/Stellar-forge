// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Verified snapshot restore executed by the node's init container on its PVC.
use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Component, Path, PathBuf}, time::Duration};
use crate::catchup::{Archive, DownloadConfig, Downloader, WorkerPool};

pub const SHA_ANNOTATION: &str = "stellar.org/snapshot-sha256";
const WORK: &str = ".stellar-bootstrap-work";
const MARKER: &str = ".stellar-bootstrap-complete";
const LOCK: &str = ".stellar-bootstrap-lock";

#[derive(Parser, Debug)]
pub struct BootstrapArgs {
    /// HTTPS or S3 URL of a gzip- or zstd-compressed tar snapshot.
    #[arg(long, env = "BACKUP_URL")]
    pub url: String,
    /// SHA-256 of the compressed bytes, supplied by a trusted publisher.
    #[arg(long, env = "SNAPSHOT_SHA256")]
    pub sha256: String,
    #[arg(long, default_value = "/data")]
    pub data_dir: PathBuf,
    #[arg(long, env = "STELLAR_CATCHUP_WORKERS", default_value_t = 8)]
    pub workers: usize,
    #[arg(long, env = "STELLAR_CATCHUP_BYTES_PER_SECOND", default_value_t = 33_554_432)]
    pub bytes_per_second: u64,
    #[arg(long, env = "STELLAR_CATCHUP_RETRIES", default_value_t = 4)]
    pub retries: u32,
    #[arg(long, env = "STELLAR_CATCHUP_CHUNK_BYTES", default_value_t = 8_388_608)]
    pub chunk_bytes: u64,
    #[arg(long, env = "STELLAR_CATCHUP_TIMEOUT_SECONDS", default_value_t = 300)]
    pub timeout_seconds: u64,
    /// Upper bound on extracted regular file bytes (default: 1 TiB).
    #[arg(long, env = "STELLAR_SNAPSHOT_MAX_UNPACKED_BYTES", default_value_t = 1_099_511_627_776)]
    pub max_unpacked_bytes: u64,
}

pub fn validate_source(source: &str, digest: &str) -> Result<()> {
    ensure!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "snapshot SHA-256 must contain exactly 64 hexadecimal characters");
    let url = url::Url::parse(source).context("snapshot URL must be absolute")?;
    ensure!(matches!(url.scheme(), "https" | "s3") && url.host_str().is_some(),
        "snapshot URL must use HTTPS or S3");
    ensure!(url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
        "snapshot URL cannot contain user information or a fragment");
    ensure!(url.path().ends_with(".tar.gz") || url.path().ends_with(".tar.zst"),
        "verified bootstrap requires a .tar.gz or .tar.zst snapshot");
    if url.scheme() == "https" {
        ensure!(url.port_or_known_default() == Some(443), "snapshot HTTPS URL must use port 443");
    }
    if url.scheme() == "s3" {
        ensure!(url.query().is_none() && url.port().is_none(), "S3 snapshot URL cannot contain a query or port");
    }
    Ok(())
}

/// The lock spans download and publication; a restarted init resumes a durable
/// installation manifest instead of treating partially installed data as ready.
pub async fn run_bootstrap(args: BootstrapArgs) -> Result<()> {
    validate_source(&args.url, &args.sha256)?;
    ensure!((1..=64).contains(&args.workers) && args.bytes_per_second > 0,
        "bootstrap workers must be 1..=64 and bandwidth must be positive");
    ensure!(args.retries <= 16 && (1..=67_108_864).contains(&args.chunk_bytes)
        && (1..=3600).contains(&args.timeout_seconds) && args.max_unpacked_bytes > 0,
        "invalid bootstrap retry, chunk, timeout, or extraction limit");
    fs::create_dir_all(&args.data_dir)?;
    ensure!(fs::symlink_metadata(&args.data_dir)?.file_type().is_dir(), "data directory must not be a symlink");
    let lock_path = args.data_dir.join(LOCK);
    reject_symlink(&lock_path)?;
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path)?;
    lock.try_lock().context("another bootstrap is using this PVC")?;
    let digest = args.sha256.to_ascii_lowercase();
    if completed(&args.data_dir, &digest)? { return Ok(()); }
    if args.data_dir.join(WORK).join("journal.json").exists() {
        publish(&args.data_dir, &digest)?;
        return Ok(());
    }
    check_empty_root(&args.data_dir)?;
    let work = args.data_dir.join(WORK);
    reject_symlink(&work)?;
    // Without a journal no data has been published. Discard only our private
    // incomplete staging area, then start a fresh verified attempt.
    if work.exists() { fs::remove_dir_all(&work)?; }
    fs::create_dir(&work)?;
    let url = url::Url::parse(&args.url)?;
    let zstd_format = url.path().ends_with(".tar.zst");
    let bearer = std::env::var("BEARER_TOKEN").ok();
    let source = if url.scheme() == "s3" {
        ensure!(bearer.is_none(), "BEARER_TOKEN applies only to HTTPS snapshots");
        use aws_sdk_s3::presigning::PresigningConfig;
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION")).unwrap_or_else(|_| "us-east-1".into());
        let settings = aws_config::from_env().region(aws_config::Region::new(region)).load().await;
        let s3 = aws_sdk_s3::Client::new(&settings);
        let key = urlencoding::decode(url.path().trim_start_matches('/'))?;
        let signed = s3.get_object().bucket(url.host_str().unwrap()).key(key.as_ref())
            .presigned(PresigningConfig::expires_in(Duration::from_secs(86_400))?).await
            .map_err(|_| anyhow::anyhow!("unable to presign S3 snapshot"))?;
        signed.uri().to_string()
    } else { args.url.clone() };
    let config = DownloadConfig { chunk_bytes: args.chunk_bytes, retries: args.retries,
        request_timeout: Duration::from_secs(args.timeout_seconds), ..DownloadConfig::default() };
    let downloader = Downloader::with_bearer(WorkerPool::new(args.workers, args.bytes_per_second)?, config, bearer.as_deref())?;
    let archive = downloader.download(Archive { url: source, destination: work.join("archive.tar.gz"), sha256: digest.clone() })
        .await.map_err(|_| anyhow::anyhow!("snapshot download or SHA-256 verification failed"))?;
    let root = args.data_dir.clone();
    tokio::task::spawn_blocking(move || {
        // Retain the cross-process lock if the async caller is cancelled while
        // filesystem publication is still running on the blocking pool.
        let _lock = lock;
        stage(&archive, &root, &digest, args.max_unpacked_bytes, zstd_format)?;
        publish(&root, &digest)
    }).await.context("bootstrap extraction task failed")??;
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct Journal { sha256: String, entries: Vec<String> }

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(!meta.file_type().is_symlink(), "bootstrap metadata cannot be a symlink"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn completed(root: &Path, digest: &str) -> Result<bool> {
    let path = root.join(MARKER);
    reject_symlink(&path)?;
    match File::open(path) {
        Ok(file) => {
            let mut recorded = String::new();
            file.take(65).read_to_string(&mut recorded)?;
            ensure!(recorded == digest, "PVC contains a different snapshot; use a fresh PVC");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn check_empty_root(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        ensure!(name == WORK || name == LOCK || name == "lost+found",
            "PVC already contains data without a completed matching restore; use a fresh PVC");
    }
    Ok(())
}

fn valid_top(name: &str) -> bool {
    !name.is_empty() && name != "lost+found" && !name.to_ascii_lowercase().starts_with(".stellar-bootstrap-")
        && Path::new(name).components().count() == 1
        && matches!(Path::new(name).components().next(), Some(Component::Normal(_)))
}

fn sync_dir(path: &Path) -> Result<()> {
    // Production init containers run on Linux. Directory fsync is unavailable
    // through std on Windows, where local source tests use best-effort durability.
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_record(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("pending");
    reject_symlink(&temporary)?;
    let mut file = OpenOptions::new().create(true).truncate(true).write(true).open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    sync_dir(path.parent().unwrap())
}

fn stage(archive_path: &Path, root: &Path, digest: &str, limit: u64, zstd_format: bool) -> Result<()> {
    let work = root.join(WORK);
    let content = work.join("content");
    fs::create_dir(&content)?;
    let file = File::open(archive_path)?;
    let decoder: Box<dyn Read> = if zstd_format {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else { Box::new(flate2::read::MultiGzDecoder::new(file)) };
    let stream_limit = limit.checked_add(103_448_576).context("extracted byte limit is too large")?;
    let mut archive = tar::Archive::new(decoder.take(stream_limit));
    let mut top = BTreeSet::new();
    let mut unpacked = 0_u64;
    let mut entry_count = 0_u32;
    for entry in archive.entries()? {
        entry_count += 1;
        ensure!(entry_count <= 100_000, "snapshot exceeds archive entry limit");
        let mut entry = entry?;
        ensure!(entry.header().entry_type().is_file() || entry.header().entry_type().is_dir(),
            "snapshot links and special files are not supported");
        let path = entry.path()?.into_owned();
        let mut parts = Vec::new();
        for component in path.components() {
            match component { Component::CurDir => (), Component::Normal(name) => parts.push(name.to_owned()),
                _ => bail!("snapshot contains an unsafe archive path") }
        }
        ensure!(parts.len() <= 128, "snapshot path is too deeply nested");
        if parts.is_empty() { ensure!(entry.header().entry_type().is_dir(), "invalid empty archive path"); continue; }
        let first = parts[0].to_str().context("snapshot entry name must be UTF-8")?;
        ensure!(valid_top(first), "snapshot uses a reserved bootstrap path");
        top.insert(first.to_owned());
        unpacked = unpacked.checked_add(entry.size()).context("snapshot size overflow")?;
        ensure!(unpacked <= limit, "snapshot exceeds extracted byte limit");
        ensure!(entry.unpack_in(&content)?, "snapshot entry escapes staging directory");
    }
    // Read through the compression trailer so truncated/corrupt archives
    // cannot be published just because tar reached its end marker first.
    let mut decoder = archive.into_inner();
    std::io::copy(&mut decoder, &mut std::io::sink())?;
    ensure!(decoder.limit() > 0, "snapshot exceeds decompressed stream limit");
    ensure!(!top.is_empty(), "snapshot is empty");
    sync_tree(&content)?;
    let journal = Journal { sha256: digest.into(), entries: top.into_iter().collect() };
    let journal_bytes = serde_json::to_vec(&journal)?;
    ensure!(journal_bytes.len() <= 16 * 1024 * 1024, "bootstrap publication journal is too large");
    write_record(&work.join("journal.json"), &journal_bytes)?;
    sync_dir(root)?;
    Ok(())
}

fn sync_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() { sync_tree(&entry.path())?; }
        else { File::open(entry.path())?.sync_all()?; }
    }
    sync_dir(path)
}

fn publish(root: &Path, digest: &str) -> Result<()> {
    let work = root.join(WORK);
    reject_symlink(&work)?;
    let journal_path = work.join("journal.json");
    reject_symlink(&journal_path)?;
    // A malformed on-volume journal must not trigger an unbounded allocation.
    let mut bytes = Vec::new();
    File::open(journal_path)?.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024 * 1024, "bootstrap publication journal is too large");
    let journal: Journal = serde_json::from_slice(&bytes)?;
    ensure!(journal.sha256 == digest && !journal.entries.is_empty(), "staged snapshot digest does not match configuration");
    let entries: BTreeSet<String> = journal.entries.iter().cloned().collect();
    ensure!(entries.len() == journal.entries.len() && journal.entries.iter().all(|name| valid_top(name)), "invalid bootstrap publication journal");
    for entry in fs::read_dir(root)? {
        let name = entry?.file_name().into_string().map_err(|_| anyhow::anyhow!("unexpected PVC entry"))?;
        if name == ".stellar-bootstrap-complete.pending" {
            ensure!(fs::symlink_metadata(root.join(&name))?.file_type().is_file(), "pending completion marker must be a regular file");
        } else {
            ensure!(name == WORK || name == LOCK || name == "lost+found" || entries.contains(&name), "PVC contains unrelated data during restore");
        }
    }
    let content = work.join("content");
    reject_symlink(&content)?;
    for name in &journal.entries {
        let source = content.join(name);
        let target = root.join(name);
        reject_symlink(&source)?;
        reject_symlink(&target)?;
        match (source.try_exists()?, target.try_exists()?) {
            (true, false) => { fs::rename(&source, &target)?; sync_dir(root)?; sync_dir(&content)?; }
            (false, true) => (), // A previous attempt completed this atomic move.
            _ => bail!("snapshot publication conflict or missing staged entry"),
        }
    }
    write_record(&root.join(MARKER), digest.as_bytes())?;
    // Completion is durable before staging is removed. An interrupted cleanup
    // is harmless: the next init sees the matching completion marker.
    fs::remove_dir_all(work)?;
    sync_dir(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn archive_bytes(name: &str, linked: bool) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o600);
        if linked {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("../outside").unwrap();
            header.set_size(0);
            header.set_cksum();
            builder.append_data(&mut header, name, std::io::empty()).unwrap();
        } else {
            header.set_size(4);
            header.set_cksum();
            builder.append_data(&mut header, name, &b"data"[..]).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn gzip_and_zstd_extract_fully_before_publication() {
        for zstd_format in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            fs::create_dir(root.join(WORK)).unwrap();
            let tar = archive_bytes("ledger/db", false);
            let compressed = if zstd_format { zstd::stream::encode_all(&tar[..], 1).unwrap() }
                else {
                    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(&tar).unwrap();
                    encoder.finish().unwrap()
                };
            let archive = root.join(WORK).join("archive");
            fs::write(&archive, compressed).unwrap();
            stage(&archive, root, DIGEST, 1024, zstd_format).unwrap();
            assert!(!root.join("ledger").exists());
            assert!(!root.join(MARKER).exists());
            publish(root, DIGEST).unwrap();
            assert_eq!(fs::read(root.join("ledger/db")).unwrap(), b"data");
            assert!(completed(root, DIGEST).unwrap());
        }
    }

    #[test]
    fn concatenated_gzip_members_form_one_complete_tar_stream() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join(WORK)).unwrap();
        let tar = archive_bytes("ledger/db", false);
        let mut compressed = Vec::new();
        for part in tar.chunks(tar.len() / 2) {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(part).unwrap();
            compressed.extend(encoder.finish().unwrap());
        }
        let archive = root.join(WORK).join("archive");
        fs::write(&archive, compressed).unwrap();
        stage(&archive, root, DIGEST, 1024, false).unwrap();
        publish(root, DIGEST).unwrap();
        assert_eq!(fs::read(root.join("ledger/db")).unwrap(), b"data");
        assert!(completed(root, DIGEST).unwrap());
    }

    #[test]
    fn archive_links_reserved_paths_and_size_overflows_never_publish() {
        for (name, linked, limit) in [("ledger/link", true, 1024), (MARKER, false, 1024), ("ledger/db", false, 3)] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            fs::create_dir(root.join(WORK)).unwrap();
            let tar = archive_bytes(name, linked);
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&tar).unwrap();
            let archive = root.join(WORK).join("archive");
            fs::write(&archive, encoder.finish().unwrap()).unwrap();
            assert!(stage(&archive, root, DIGEST, limit, false).is_err());
            assert!(!root.join(WORK).join("journal.json").exists());
            assert!(!root.join(MARKER).exists());
        }
    }

    #[test]
    fn resume_moves_only_remaining_entries_and_marks_completion_last() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let work = root.join(WORK);
        fs::create_dir_all(work.join("content")).unwrap();
        fs::write(root.join("already-moved.db"), b"first").unwrap();
        fs::write(work.join("content/remaining.db"), b"second").unwrap();
        let journal = Journal { sha256: DIGEST.into(), entries: vec!["already-moved.db".into(), "remaining.db".into()] };
        fs::write(work.join("journal.json"), serde_json::to_vec(&journal).unwrap()).unwrap();
        // Simulate a crash after the completion record was fsynced but before
        // its atomic rename. The matching journal authorizes recovery.
        fs::write(root.join(".stellar-bootstrap-complete.pending"), DIGEST).unwrap();
        assert!(!completed(root, DIGEST).unwrap());
        publish(root, DIGEST).unwrap();
        assert_eq!(fs::read(root.join("remaining.db")).unwrap(), b"second");
        assert!(completed(root, DIGEST).unwrap());
        assert!(!work.exists());
        assert!(completed(root, &"b".repeat(64)).is_err());
    }

    #[test]
    fn publication_conflict_cannot_mark_partial_data_complete() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let work = root.join(WORK);
        fs::create_dir_all(work.join("content")).unwrap();
        fs::write(root.join("db"), b"old").unwrap();
        fs::write(work.join("content/db"), b"new").unwrap();
        let journal = Journal { sha256: DIGEST.into(), entries: vec!["db".into()] };
        fs::write(work.join("journal.json"), serde_json::to_vec(&journal).unwrap()).unwrap();
        assert!(publish(root, DIGEST).is_err());
        assert!(!root.join(MARKER).exists());
        assert_eq!(fs::read(root.join("db")).unwrap(), b"old");
    }

    #[test]
    fn sources_reserved_paths_and_unrelated_data_are_rejected() {
        assert!(validate_source("https://host/snapshot.tar.gz", DIGEST).is_ok());
        assert!(validate_source("s3://bucket/snapshot.tar.gz", DIGEST).is_ok());
        assert!(validate_source("http://host/snapshot.tar.gz", DIGEST).is_err());
        assert!(validate_source("https://host/snapshot.tar.zst", DIGEST).is_ok());
        assert!(validate_source("https://host/snapshot.zip", DIGEST).is_err());
        assert!(validate_source("https://host/snapshot.tar.gz", "invalid").is_err());
        assert!(!valid_top("../escape"));
        assert!(!valid_top(MARKER));
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("lost+found")).unwrap();
        assert!(check_empty_root(temp.path()).is_ok());
        fs::write(temp.path().join("existing.db"), b"do not replace").unwrap();
        assert!(check_empty_root(temp.path()).is_err());
    }
}

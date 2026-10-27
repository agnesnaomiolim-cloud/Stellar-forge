// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
use super::WorkerPool;
use anyhow::{bail, ensure, Context, Result};
use reqwest::{header, Client, Response, StatusCode};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    task::JoinSet,
};

#[derive(Debug, thiserror::Error)]
#[error("archive body interrupted")]
struct BodyInterrupted;

#[derive(Clone)]
pub struct DownloadConfig {
    pub chunk_bytes: u64,
    pub retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// Maximum connection or network-read inactivity; excludes egress pacing.
    pub request_timeout: Duration,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            chunk_bytes: 8 * 1024 * 1024,
            retries: 4,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(8),
            request_timeout: Duration::from_secs(300),
        }
    }
}

/// A trusted manifest supplies the SHA-256 of the compressed archive bytes.
#[derive(Clone)]
pub struct Archive {
    pub url: String,
    pub destination: PathBuf,
    pub sha256: String,
}

#[derive(Clone)]
pub struct Downloader {
    client: Client,
    pool: Arc<WorkerPool>,
    config: DownloadConfig,
}

impl Downloader {
    pub fn new(pool: Arc<WorkerPool>, config: DownloadConfig) -> Result<Self> {
        Self::with_bearer(pool, config, None)
    }

    /// Optional authenticated HTTPS archive access. The header is sensitive so
    /// debug formatting does not expose credentials; errors never echo tokens.
    pub fn with_bearer(
        pool: Arc<WorkerPool>,
        config: DownloadConfig,
        token: Option<&str>,
    ) -> Result<Self> {
        ensure!(config.chunk_bytes > 0, "chunk size must be positive");
        ensure!(
            !config.request_timeout.is_zero(),
            "request timeout must be positive"
        );
        let mut headers = header::HeaderMap::new();
        if let Some(token) = token {
            ensure!(!token.trim().is_empty(), "archive bearer token is empty");
            let mut value = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| anyhow::anyhow!("invalid archive authorization header"))?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        // Feature unification may enable compression through another reqwest
        // user. Range offsets and the trusted digest refer to wire bytes.
        let client = Client::builder()
            .default_headers(headers)
            // A total request deadline also counts deliberate limiter waits.
            // Large fallback bodies must be allowed to outlive that deadline
            // while each network operation still has a finite timeout.
            .connect_timeout(config.request_timeout)
            .read_timeout(config.request_timeout)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()?;
        Ok(Self {
            client,
            pool,
            config,
        })
    }

    /// Download to a unique temporary file and publish only after verification.
    /// Dropping this future aborts in-flight tasks. Temporary-file deletion on
    /// cancellation is best effort (Windows may still have an open worker handle).
    pub async fn download(&self, archive: Archive) -> Result<PathBuf> {
        let expected = hex::decode(&archive.sha256).context("invalid SHA-256")?;
        ensure!(expected.len() == 32, "expected a 32-byte SHA-256 digest");
        let parent = archive
            .destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).await?;
        let staging_parent = parent.to_owned();
        let temporary = tokio::task::spawn_blocking(move || {
            tempfile::NamedTempFile::new_in(staging_parent).map(|file| file.into_temp_path())
        })
        .await
        .context("temporary file creation task failed")??;
        let path = temporary.to_path_buf();
        // Probe with GET rather than HEAD: signed cloud GET URLs may reject HEAD.
        let _slot = self.pool.slots.acquire().await?;
        let response = self.request(&archive.url, 0, 0, None).await?;
        let total;
        if response.status() == StatusCode::OK {
            total = response
                .content_length()
                .context("archive requires Content-Length")?;
            let mut response = response;
            let mut attempt = 0;
            loop {
                match self.write_body(response, &path, 0, total).await {
                    Err(error)
                        if error.is::<BodyInterrupted>() && attempt < self.config.retries =>
                    {
                        tokio::time::sleep(self.backoff(attempt)).await;
                        attempt += 1;
                        response = self.request(&archive.url, 0, 0, None).await?;
                        ensure!(
                            response.status() == StatusCode::OK,
                            "archive range support changed during retry"
                        );
                    }
                    result => {
                        result?;
                        break;
                    }
                }
            }
            drop(_slot);
        } else {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .context("missing Content-Range")?
                .to_str()?;
            total = range
                .strip_prefix("bytes 0-0/")
                .context("invalid probe Content-Range")?
                .parse::<u64>()?;
            ensure!(total > 0, "empty archive range");
            let etag = response
                .headers()
                .get(header::ETAG)
                .and_then(|h| h.to_str().ok())
                .filter(|s| !s.starts_with("W/"))
                .map(str::to_owned);
            // Validate the probe body, including truncation and oversized bodies.
            let mut probe = response;
            let mut attempt = 0;
            loop {
                match self.write_body(probe, &path, 0, 1).await {
                    Err(error)
                        if error.is::<BodyInterrupted>() && attempt < self.config.retries =>
                    {
                        tokio::time::sleep(self.backoff(attempt)).await;
                        attempt += 1;
                        probe = self.request(&archive.url, 0, 0, None).await?;
                        ensure!(
                            probe.status() == StatusCode::PARTIAL_CONTENT
                                && probe
                                    .headers()
                                    .get(header::CONTENT_RANGE)
                                    .and_then(|h| h.to_str().ok())
                                    == Some(format!("bytes 0-0/{total}").as_str()),
                            "archive changed during probe retry"
                        );
                    }
                    result => {
                        result?;
                        break;
                    }
                }
            }
            drop(_slot);
            let output = OpenOptions::new().write(true).open(&path).await?;
            output.set_len(total).await?;
            drop(output);
            let mut tasks = JoinSet::new();
            let mut next = 0;
            while next < total || !tasks.is_empty() {
                while next < total && tasks.len() < self.pool.workers {
                    let start = next;
                    let end = start
                        .saturating_add(self.config.chunk_bytes - 1)
                        .min(total - 1);
                    next = end + 1;
                    let (worker, url, file, tag) = (
                        self.clone(),
                        archive.url.clone(),
                        path.clone(),
                        etag.clone(),
                    );
                    tasks.spawn(async move {
                        worker
                            .chunk(&url, &file, start, end, total, tag.as_deref())
                            .await
                    });
                }
                if let Some(result) = tasks.join_next().await {
                    let result = result
                        .map_err(anyhow::Error::from)
                        .and_then(|result| result);
                    if let Err(error) = result {
                        // Reap all workers before the staging path is dropped.
                        tasks.shutdown().await;
                        return Err(error);
                    }
                }
            }
        }
        let mut input = fs::File::open(&path).await?;
        ensure!(
            input.metadata().await?.len() == total,
            "archive length mismatch"
        );
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        ensure!(
            hasher.finalize().as_slice() == expected.as_slice(),
            "archive SHA-256 mismatch"
        );
        drop(input);
        fs::rename(&path, &archive.destination).await?;
        Ok(archive.destination)
    }

    async fn request(
        &self,
        url: &str,
        start: u64,
        end: u64,
        etag: Option<&str>,
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            let mut request = self
                .client
                .get(url)
                .header(header::ACCEPT_ENCODING, "identity")
                .header(header::RANGE, format!("bytes={start}-{end}"));
            if let Some(tag) = etag {
                request = request.header(header::IF_RANGE, tag);
            }
            let result = request.send().await;
            let transient = match &result {
                Ok(response) => {
                    response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS
                }
                Err(error) => error.is_timeout() || error.is_connect() || error.is_body(),
            };
            if transient && attempt < self.config.retries {
                drop(result);
                tokio::time::sleep(self.backoff(attempt)).await;
                attempt += 1;
                continue;
            }
            // Avoid returning reqwest errors containing presigned credentials.
            let response = result.map_err(|_| anyhow::anyhow!("archive HTTP transport failed"))?;
            ensure!(
                response.status() == StatusCode::OK
                    || response.status() == StatusCode::PARTIAL_CONTENT,
                "archive HTTP status {}",
                response.status()
            );
            ensure!(
                response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .map_or(true, |h| h == "identity"),
                "encoded range responses are unsupported"
            );
            return Ok(response);
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.config
            .initial_backoff
            .saturating_mul(2u32.saturating_pow(attempt))
            .min(self.config.max_backoff)
    }

    async fn chunk(
        &self,
        url: &str,
        path: &Path,
        start: u64,
        end: u64,
        total: u64,
        etag: Option<&str>,
    ) -> Result<()> {
        let _slot = self.pool.slots.acquire().await?;
        let mut attempt = 0;
        loop {
            let response = self.request(url, start, end, etag).await?;
            ensure!(
                response.status() == StatusCode::PARTIAL_CONTENT,
                "server ignored range or archive changed"
            );
            let expected = format!("bytes {start}-{end}/{total}");
            ensure!(
                response
                    .headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    == Some(expected.as_str()),
                "incorrect Content-Range"
            );
            match self
                .write_body(response, path, start, end - start + 1)
                .await
            {
                Err(error) if error.is::<BodyInterrupted>() && attempt < self.config.retries => {
                    tokio::time::sleep(self.backoff(attempt)).await;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn write_body(
        &self,
        mut response: Response,
        path: &Path,
        offset: u64,
        expected: u64,
    ) -> Result<()> {
        if let Some(length) = response.content_length() {
            ensure!(length == expected, "incorrect Content-Length");
        }
        let mut output = OpenOptions::new().write(true).open(path).await?;
        output.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut written = 0u64;
        while let Some(bytes) = response.chunk().await.map_err(|_| BodyInterrupted)? {
            if bytes.len() as u64 > expected - written {
                bail!("oversized archive response");
            }
            self.pool.account(bytes.len()).await;
            output.write_all(&bytes).await?;
            written += bytes.len() as u64;
        }
        if written != expected {
            return Err(BodyInterrupted.into());
        }
        output.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    // A real local HTTP peer checks actual Range requests and response parsing.
    async fn server(
        mode: &'static str,
    ) -> (
        String,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let peak = Arc::new(AtomicUsize::new(0));
        let highest = peak.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let overlap = Arc::new(tokio::sync::Barrier::new(2));
        let handle = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (mut socket, _) = tokio::select! {
                    completed = connections.join_next(), if !connections.is_empty() => {
                        completed.unwrap().expect("HTTP test handler panicked");
                        continue;
                    }
                    accepted = listener.accept() => accepted.unwrap(),
                };
                let (count, active, highest) = (count.clone(), active.clone(), highest.clone());
                let overlap = overlap.clone();
                connections.spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).await.unwrap() == 0 { break; }
                    request.push(byte[0]);
                }
                let index = count.fetch_add(1, Ordering::SeqCst);
                if mode == "retry" && index == 0 {
                    socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    return;
                }
                let text = String::from_utf8_lossy(&request).to_lowercase();
                assert!(text.contains("accept-encoding: identity"));
                if mode == "authenticated" {
                    assert!(text.lines().any(|line| line == "authorization: bearer fixture"));
                }
                let range = text.lines().find_map(|line| line.strip_prefix("range: bytes=")).unwrap();
                let (start, end) = range.split_once('-').unwrap();
                let (start, end) = (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap());
                let data = b"abcdefghijklmnop";
                highest.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                if mode == "overlap" && (index == 1 || index == 2) {
                    // Deterministically overlap the first pair of ranges; a
                    // sequential downloader cannot satisfy this rendezvous.
                    overlap.wait().await;
                }
                let (status, mut header, body) = if mode == "fallback" {
                    ("200 OK", String::new(), data.to_vec())
                } else {
                    let actual_start = if mode == "wrong" && index > 0 { start + 1 } else { start };
                    ("206 Partial Content", format!("Content-Range: bytes {actual_start}-{end}/16\r\n"), data[start..=end].to_vec())
                };
                if mode == "encoded" { header.push_str("Content-Encoding: gzip\r\n"); }
                let declared_length = if mode == "wrong-length" { body.len() + 1 } else { body.len() };
                let response = format!("HTTP/1.1 {status}\r\n{header}Content-Length: {declared_length}\r\nConnection: close\r\n\r\n");
                // Rejected responses may be closed before their body is sent.
                if socket.write_all(response.as_bytes()).await.is_err() {
                    active.fetch_sub(1, Ordering::SeqCst);
                    return;
                }
                let truncate = (mode == "body-reset" && index == 1) || (mode == "probe-reset" && index == 0);
                if truncate {
                    let _ = socket.write_all(&body[..body.len() - 1]).await;
                } else {
                    let _ = socket.write_all(&body).await;
                }
                active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        (format!("http://{address}/archive"), requests, peak, handle)
    }

    async fn check(mode: &'static str, checksum_valid: bool) -> (bool, usize, bool) {
        let (url, requests, peak, server) = server(mode).await;
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("archive");
        let config = DownloadConfig {
            chunk_bytes: 4,
            initial_backoff: Duration::from_millis(1),
            ..Default::default()
        };
        let downloader = Downloader::new(WorkerPool::new(2, 1_000_000).unwrap(), config).unwrap();
        let checksum = if checksum_valid {
            hex::encode(Sha256::digest(b"abcdefghijklmnop"))
        } else {
            "00".repeat(32)
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            downloader.download(Archive {
                url,
                destination: destination.clone(),
                sha256: checksum,
            }),
        )
        .await
        .expect("download timed out");
        let success = result.is_ok();
        if success {
            assert_eq!(fs::read(&destination).await.unwrap(), b"abcdefghijklmnop");
            if mode == "overlap" {
                assert_eq!(
                    peak.load(Ordering::SeqCst),
                    2,
                    "requests overlap up to the configured bound"
                );
            }
        }
        let published = destination.exists();
        server.abort();
        assert!(
            server.await.unwrap_err().is_cancelled(),
            "HTTP test server panicked"
        );
        (success, requests.load(Ordering::SeqCst), published)
    }

    #[tokio::test]
    async fn assembles_ranges_and_retries_service_errors() {
        let (success, requests, published) = check("retry", true).await;
        assert!(success && published);
        assert_eq!(requests, 6); // failed probe + probe + four chunks
    }

    #[tokio::test]
    async fn supports_servers_ignoring_initial_range() {
        assert_eq!(check("fallback", true).await, (true, 1, true));
    }

    #[test]
    fn malformed_bearer_headers_are_rejected_without_echoing_the_token() {
        for token in ["", "\r\nX-Injected: fixture"] {
            assert!(Downloader::with_bearer(
                WorkerPool::new(1, 1024).unwrap(), DownloadConfig::default(), Some(token),
            ).is_err());
        }
    }

    #[tokio::test]
    async fn authenticated_requests_include_the_header_on_probe_and_chunks() {
        let (url, requests, _, server) = server("authenticated").await;
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("archive");
        let downloader = Downloader::with_bearer(
            WorkerPool::new(2, 1_000_000).unwrap(),
            DownloadConfig { chunk_bytes: 4, ..Default::default() },
            Some("fixture"),
        ).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            downloader.download(Archive {
                url, destination: destination.clone(),
                sha256: hex::encode(Sha256::digest(b"abcdefghijklmnop")),
            }),
        ).await.expect("authenticated download timed out");
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        result.unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 5);
        assert_eq!(fs::read(destination).await.unwrap(), b"abcdefghijklmnop");
    }

    #[tokio::test]
    async fn egress_pacing_can_outlive_the_network_timeout() {
        let (url, requests, _, server) = server("fallback").await;
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("archive");
        let downloader = Downloader::new(
            WorkerPool::new(1, 16).unwrap(),
            DownloadConfig {
                retries: 0,
                request_timeout: Duration::from_millis(200),
                ..Default::default()
            },
        )
        .unwrap();
        // A sixteen-byte response takes at least a second under this budget,
        // even though the local peer sends every byte immediately.
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            downloader.download(Archive {
                url,
                destination: destination.clone(),
                sha256: hex::encode(Sha256::digest(b"abcdefghijklmnop")),
            }),
        )
        .await
        .expect("paced download timed out");
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        result.unwrap();
        assert_eq!(fs::read(destination).await.unwrap(), b"abcdefghijklmnop");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn range_requests_overlap_within_the_worker_bound() {
        assert_eq!(check("overlap", true).await, (true, 5, true));
    }

    #[tokio::test]
    async fn retries_interrupted_probe_and_chunk_bodies() {
        for mode in ["body-reset", "probe-reset"] {
            let (success, requests, published) = check(mode, true).await;
            assert!(success && published);
            assert_eq!(requests, 6);
        }
    }

    #[tokio::test]
    async fn rejects_wrong_ranges_and_does_not_publish() {
        let (success, _, published) = check("wrong", true).await;
        assert!(!success && !published);
    }

    #[tokio::test]
    async fn rejects_corrupt_archives_and_does_not_publish() {
        let (success, _, published) = check("range", false).await;
        assert!(!success && !published);
    }

    #[tokio::test]
    async fn rejects_transformed_or_misdeclared_wire_bytes() {
        for mode in ["encoded", "wrong-length"] {
            let (success, _, published) = check(mode, true).await;
            assert!(!success && !published);
        }
    }

    #[test]
    fn caps_exponential_backoff() {
        let downloader =
            Downloader::new(WorkerPool::new(1, 1).unwrap(), DownloadConfig::default()).unwrap();
        assert_eq!(downloader.backoff(0), Duration::from_millis(250));
        assert_eq!(downloader.backoff(1), Duration::from_millis(500));
        assert_eq!(downloader.backoff(100), Duration::from_secs(8));
    }
}

// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Source-only benchmark driver. See docs/development/catchup-downloader.md.
use controller::catchup::{Archive, DownloadConfig, Downloader, WorkerPool};
use std::{path::PathBuf, time::Instant};
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(args.len() == 6, "usage: catchup-download MANIFEST.tsv OUTPUT WORKERS BYTES_PER_SECOND CHUNK_BYTES");
    let workers: usize = args[3].parse()?;
    let config = DownloadConfig { chunk_bytes: args[5].parse()?, ..Default::default() };
    let downloader = Downloader::new(WorkerPool::new(workers, args[4].parse()?)?, config)?;
    let manifest = tokio::fs::read_to_string(&args[1]).await?;
    let mut archives = Vec::new();
    for (index, line) in manifest.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).enumerate() {
        let (digest, url) = line.split_once('\t').ok_or_else(|| anyhow::anyhow!("manifest requires SHA256<TAB>URL"))?;
        archives.push(Archive { sha256: digest.to_owned(), url: url.to_owned(),
            destination: PathBuf::from(&args[2]).join(format!("{index}.archive")) });
    }
    let count = archives.len();
    let start = Instant::now();
    let mut tasks = JoinSet::new();
    for archive in archives {
        while tasks.len() >= workers { tasks.join_next().await.unwrap()??; }
        let downloader = downloader.clone();
        tasks.spawn(async move { downloader.download(archive).await });
    }
    while let Some(result) = tasks.join_next().await { result??; }
    println!("archives,workers,elapsed_seconds\n{count},{workers},{:.3}", start.elapsed().as_secs_f64());
    Ok(())
}

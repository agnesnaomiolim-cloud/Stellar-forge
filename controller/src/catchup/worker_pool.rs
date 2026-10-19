// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{Mutex, Semaphore},
    time::{sleep_until, Instant},
};

/// Share one instance among every downloader using the same egress allocation.
/// For separate pods, divide the node allocation between replicas or enforce a
/// node-level traffic shaper; an in-process limiter cannot police other pods.
pub struct WorkerPool {
    pub(crate) slots: Arc<Semaphore>,
    next_byte: Mutex<Instant>,
    bytes_per_second: u64,
    pub(crate) workers: usize,
}

impl WorkerPool {
    pub fn new(workers: usize, bytes_per_second: u64) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            workers > 0 && workers <= Semaphore::MAX_PERMITS && bytes_per_second > 0,
            "workers and egress limit must be positive and workers must fit semaphore capacity"
        );
        Ok(Arc::new(Self {
            slots: Arc::new(Semaphore::new(workers)),
            next_byte: Mutex::new(Instant::now()),
            bytes_per_second,
            workers,
        }))
    }

    /// Process-wide default: eight requests and 32 MiB/s across all bootstraps.
    pub fn shared_default() -> Arc<Self> {
        static POOL: OnceLock<Arc<WorkerPool>> = OnceLock::new();
        POOL.get_or_init(|| Self::new(8, 32 * 1024 * 1024).expect("valid defaults"))
            .clone()
    }

    /// Read the allocation once per process. Every snapshot reconciler shares it.
    pub fn shared_from_env() -> anyhow::Result<Arc<Self>> {
        static POOL: OnceLock<Arc<WorkerPool>> = OnceLock::new();
        if let Some(pool) = POOL.get() {
            return Ok(pool.clone());
        }
        let workers = std::env::var("STELLAR_CATCHUP_WORKERS")
            .unwrap_or_else(|_| "8".into())
            .parse()?;
        let rate = std::env::var("STELLAR_CATCHUP_BYTES_PER_SECOND")
            .unwrap_or_else(|_| "33554432".into())
            .parse()?;
        let pool = Self::new(workers, rate)?;
        Ok(POOL.get_or_init(|| pool).clone())
    }

    pub(crate) async fn account(&self, bytes: usize) {
        // Holding this lock across the wait prevents independent workers from
        // each consuming the full budget and avoids an unbounded reservation queue.
        let mut next = self.next_byte.lock().await;
        let start = (*next).max(Instant::now());
        let duration = Duration::from_secs_f64(bytes as f64 / self.bytes_per_second as f64);
        *next = start + duration;
        sleep_until(*next).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workers_share_one_budget() {
        let pool = WorkerPool::new(2, 10_000).unwrap();
        let start = Instant::now();
        let ((), ()) = tokio::join!(pool.account(100), pool.account(100));
        assert!(start.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn invalid_limits_are_rejected() {
        assert!(WorkerPool::new(0, 1).is_err());
        assert!(WorkerPool::new(1, 0).is_err());
        assert!(Arc::ptr_eq(
            &WorkerPool::shared_default(),
            &WorkerPool::shared_default()
        ));
    }
}

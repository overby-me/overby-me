//! Bounded job queue with configurable workers for `tangled-spindle-nix`.
//!
//! Provides a bounded, async job queue that limits the number of concurrent
//! workflow executions. Matches the upstream Go spindle's `queue.go` behavior.
//!
//! # Architecture
//!
//! The queue uses a `tokio::sync::Semaphore` to limit concurrency and a
//! bounded `tokio::sync::mpsc` channel for backpressure. Each dequeued job
//! is spawned as a `tokio::task` that acquires a semaphore permit before
//! executing the workflow.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc};
use tracing::{debug, error, info, warn};

/// A boxed, pinned future representing a job to execute.
pub type Job = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// A factory function that produces a job future.
///
/// We use a boxed closure so callers can capture state and produce the
/// future lazily (only when a worker picks up the job).
pub type JobFn = Box<dyn FnOnce() -> Job + Send + 'static>;

/// Errors that can occur with the job queue.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// The queue is full and cannot accept more jobs.
    #[error("queue is full (capacity: {capacity})")]
    Full { capacity: usize },

    /// The queue has been shut down.
    #[error("queue is shut down")]
    Closed,
}

/// A bounded job queue with configurable concurrency.
///
/// Jobs are submitted via [`submit`](JobQueue::submit) and executed by
/// background worker tasks. The number of concurrently executing jobs is
/// limited by `max_jobs`. Jobs beyond `queue_size` are rejected.
pub struct JobQueue {
    /// Channel sender for submitting jobs.
    tx: mpsc::Sender<JobFn>,
    /// Queue capacity (for error messages).
    queue_size: usize,
}

impl JobQueue {
    /// Create a new job queue and spawn the dispatcher task.
    ///
    /// # Arguments
    /// * `max_jobs` — Maximum number of jobs executing concurrently.
    /// * `queue_size` — Maximum number of pending jobs waiting for a worker.
    /// * `shutdown` — Cancellation token to stop the queue.
    pub fn new(
        max_jobs: usize,
        queue_size: usize,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<JobFn>(queue_size);

        tokio::spawn(run_dispatcher(rx, max_jobs, shutdown));

        info!(max_jobs, queue_size, "job queue started");

        Self { tx, queue_size }
    }

    /// Submit a job to the queue.
    ///
    /// The job factory `f` is called when a worker picks up the job. This
    /// allows callers to defer expensive setup until execution time.
    ///
    /// Returns `QueueError::Full` if the queue is at capacity, or
    /// `QueueError::Closed` if the queue has been shut down.
    pub fn submit(&self, f: JobFn) -> Result<(), QueueError> {
        match self.tx.try_send(f) {
            Ok(()) => {
                debug!("job submitted to queue");
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    capacity = self.queue_size,
                    "job queue is full, rejecting job"
                );
                Err(QueueError::Full {
                    capacity: self.queue_size,
                })
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("job queue is closed, rejecting job");
                Err(QueueError::Closed)
            }
        }
    }
}

/// Run the dispatcher loop that pulls jobs from the channel and spawns them
/// with concurrency limiting via a semaphore.
async fn run_dispatcher(
    mut rx: mpsc::Receiver<JobFn>,
    max_jobs: usize,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let semaphore = Arc::new(Semaphore::new(max_jobs));

    loop {
        tokio::select! {
            job_fn = rx.recv() => {
                match job_fn {
                    Some(f) => {
                        let permit = semaphore.clone().acquire_owned().await;
                        match permit {
                            Ok(permit) => {
                                debug!("worker acquired permit, executing job");
                                tokio::spawn(async move {
                                    let job = f();
                                    job.await;
                                    drop(permit);
                                    debug!("job completed, permit released");
                                });
                            }
                            Err(_) => {
                                error!("semaphore closed unexpectedly");
                                break;
                            }
                        }
                    }
                    None => {
                        info!("job queue channel closed, dispatcher exiting");
                        break;
                    }
                }
            }
            () = shutdown.cancelled() => {
                info!("job queue shutdown requested");
                break;
            }
        }
    }

    rx.close();
    info!("job queue dispatcher stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn basic_job_execution() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(2, 10, shutdown.clone());

        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        queue
            .submit(Box::new(move || {
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                })
            }))
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        shutdown.cancel();
    }

    #[tokio::test]
    async fn concurrency_limiting() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(1, 10, shutdown.clone());

        let running = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        for _ in 0..5 {
            let r = running.clone();
            let m = max_concurrent.clone();
            queue
                .submit(Box::new(move || {
                    Box::pin(async move {
                        let current = r.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        r.fetch_sub(1, Ordering::SeqCst);
                    })
                }))
                .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(max_concurrent.load(Ordering::SeqCst), 1);
        assert_eq!(running.load(Ordering::SeqCst), 0);

        shutdown.cancel();
    }

    #[tokio::test]
    async fn queue_full_rejection() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(1, 1, shutdown.clone());

        let barrier = Arc::new(tokio::sync::Notify::new());
        let b = barrier.clone();
        queue
            .submit(Box::new(move || {
                Box::pin(async move {
                    b.notified().await;
                })
            }))
            .unwrap();

        // Give the dispatcher time to pick up the first job.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Fill the channel buffer.
        queue.submit(Box::new(|| Box::pin(async {}))).unwrap();

        // This should be rejected.
        let result = queue.submit(Box::new(|| Box::pin(async {})));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), QueueError::Full { .. }));

        barrier.notify_one();
        shutdown.cancel();
    }

    #[tokio::test]
    async fn stress_many_concurrent_jobs() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(4, 100, shutdown.clone());

        let completed = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicUsize::new(0));

        let total_jobs = 50;
        for _ in 0..total_jobs {
            let c = completed.clone();
            let m = max_concurrent.clone();
            let r = running.clone();
            queue
                .submit(Box::new(move || {
                    Box::pin(async move {
                        let current = r.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(current, Ordering::SeqCst);
                        // Simulate work
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        r.fetch_sub(1, Ordering::SeqCst);
                        c.fetch_add(1, Ordering::SeqCst);
                    })
                }))
                .unwrap();
        }

        // Wait for all jobs to complete
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(completed.load(Ordering::SeqCst), total_jobs);
        // Concurrency should never exceed max_jobs=4
        assert!(max_concurrent.load(Ordering::SeqCst) <= 4);
        assert_eq!(running.load(Ordering::SeqCst), 0);

        shutdown.cancel();
    }

    #[tokio::test]
    async fn stress_queue_saturation_and_recovery() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(1, 5, shutdown.clone());

        let barrier = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(AtomicUsize::new(0));

        // Block the single worker
        let b = barrier.clone();
        queue
            .submit(Box::new(move || {
                Box::pin(async move {
                    b.notified().await;
                })
            }))
            .unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;

        // Fill the queue buffer (capacity=5)
        for _ in 0..5 {
            let c = completed.clone();
            queue
                .submit(Box::new(move || {
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::SeqCst);
                    })
                }))
                .unwrap();
        }

        // Queue should now be full — next submit should fail
        let result = queue.submit(Box::new(|| Box::pin(async {})));
        assert!(matches!(result, Err(QueueError::Full { capacity: 5 })));

        // Unblock the worker — queue should drain
        barrier.notify_one();
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(completed.load(Ordering::SeqCst), 5);

        // Queue should accept new jobs again
        let c = completed.clone();
        queue
            .submit(Box::new(move || {
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                })
            }))
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(completed.load(Ordering::SeqCst), 6);

        shutdown.cancel();
    }

    #[tokio::test]
    async fn shutdown_stops_dispatcher() {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let queue = JobQueue::new(2, 10, shutdown.clone());

        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        queue
            .submit(Box::new(move || {
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                })
            }))
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        shutdown.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

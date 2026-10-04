//! The export queue: one worker thread runs the jobs one after the other.
//!
//! Only one job runs at a time. A job that starts while another job runs
//! waits in the queue. It sends its `started` event when it begins. A
//! cancel of a queued job is possible: the job then begins, sees the
//! cancel before its first photo, and finishes with `cancelled: true`.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::{CommandError, ErrorKind};

/// The id of an export job. Ids start at 1 and are unique while the
/// application runs.
pub type JobId = u64;

/// The work of a job. It gets the control of the job.
pub type JobTask = Box<dyn FnOnce(&JobControl) + Send + 'static>;

type JobMap = Arc<Mutex<HashMap<JobId, Arc<AtomicBool>>>>;

/// Locks the job map. The map holds only cancel flags, and no operation
/// on it can leave it half changed, so a poisoned lock is safe to use.
fn lock(jobs: &JobMap) -> MutexGuard<'_, HashMap<JobId, Arc<AtomicBool>>> {
    jobs.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The cancel flag and the registration of one job.
#[derive(Debug)]
pub struct JobControl {
    id: JobId,
    cancel: Arc<AtomicBool>,
    jobs: JobMap,
}

impl JobControl {
    /// Returns the job id.
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Returns `true` if the UI cancelled the job.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Removes the job from the list of known jobs. After this call,
    /// [`ExportManager::cancel`] gives "not found" for the job. A second
    /// call has no effect.
    pub fn retire(&self) {
        lock(&self.jobs).remove(&self.id);
    }
}

struct QueuedJob {
    control: JobControl,
    task: JobTask,
}

/// The export queue (Tauri managed state).
#[derive(Debug)]
pub struct ExportManager {
    sender: Sender<QueuedJob>,
    jobs: JobMap,
    next_id: AtomicU64,
}

impl ExportManager {
    /// Starts the worker thread.
    ///
    /// # Errors
    ///
    /// Returns the error of the operating system if the thread cannot
    /// start.
    pub fn start() -> std::io::Result<Self> {
        let (sender, receiver) = channel::<QueuedJob>();
        std::thread::Builder::new()
            .name("export-worker".into())
            .spawn(move || worker(&receiver))?;
        Ok(Self {
            sender,
            jobs: Arc::default(),
            next_id: AtomicU64::new(1),
        })
    }

    /// Adds a job to the queue and returns its id.
    ///
    /// # Errors
    ///
    /// Returns an `Internal` error if the worker thread stopped.
    pub fn submit(&self, task: JobTask) -> Result<JobId, CommandError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.jobs).insert(id, Arc::clone(&cancel));
        let control = JobControl {
            id,
            cancel,
            jobs: Arc::clone(&self.jobs),
        };
        self.sender.send(QueuedJob { control, task }).map_err(|_| {
            lock(&self.jobs).remove(&id);
            CommandError::internal("the export worker thread stopped")
        })?;
        Ok(id)
    }

    /// Requests a cancel of a job. The job stops before its next photo.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` if the job is not known or is finished.
    pub fn cancel(&self, id: JobId) -> Result<(), CommandError> {
        let jobs = lock(&self.jobs);
        let flag = jobs.get(&id).ok_or_else(|| {
            CommandError::new(
                ErrorKind::NotFound,
                format!("the export job {id} is not known or is finished"),
            )
        })?;
        flag.store(true, Ordering::Release);
        Ok(())
    }
}

fn worker(receiver: &Receiver<QueuedJob>) {
    // The loop stops when the manager (the sender) is dropped.
    while let Ok(QueuedJob { control, task }) = receiver.recv() {
        let id = control.id;
        if catch_unwind(AssertUnwindSafe(|| task(&control))).is_err() {
            tracing::error!(job = id, "an export job panicked");
        }
        control.retire();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn runs_jobs_in_order_and_cancels() {
        let manager = ExportManager::start().unwrap();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel::<(JobId, bool)>();

        // The first job waits for the gate, so the second job stays queued.
        let done = done_tx.clone();
        let first = manager
            .submit(Box::new(move |control| {
                gate_rx.recv_timeout(WAIT).unwrap();
                done.send((control.id(), control.is_cancelled())).unwrap();
            }))
            .unwrap();
        let second = manager
            .submit(Box::new(move |control| {
                done_tx
                    .send((control.id(), control.is_cancelled()))
                    .unwrap();
            }))
            .unwrap();
        assert_eq!((first, second), (1, 2));

        manager.cancel(second).unwrap();
        gate_tx.send(()).unwrap();
        assert_eq!(done_rx.recv_timeout(WAIT).unwrap(), (first, false));
        assert_eq!(done_rx.recv_timeout(WAIT).unwrap(), (second, true));

        // The worker retires a job after its task. Wait for that.
        let deadline = std::time::Instant::now() + WAIT;
        while manager.cancel(second).is_ok() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            manager.cancel(second).unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert_eq!(manager.cancel(99).unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn worker_survives_a_panic() {
        let manager = ExportManager::start().unwrap();
        manager.submit(Box::new(|_| panic!("test panic"))).unwrap();
        let (tx, rx) = mpsc::channel();
        manager
            .submit(Box::new(move |control| tx.send(control.id()).unwrap()))
            .unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), 2);
    }
}

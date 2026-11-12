//! Cancellation and progress.
//!
//! A [`Cancel`] is shared between whoever starts a job and the job itself. Stages check
//! it between rows, so a cancelled run stops within a row or so of work and returns
//! [`Error::Cancelled`]. A cancelled run never replaces the result of a stage: the
//! pipeline only stores a stage's output after the stage returned successfully.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::{Error, Result};

#[derive(Debug, Default)]
struct Shared {
    cancelled: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
}

/// A cheap to clone cancellation flag with a progress counter.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<Shared>);

impl Cancel {
    /// A fresh token, not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the job to stop. Every clone sees it.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`Cancel::cancel`] was called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }

    /// Returns [`Error::Cancelled`] once the token is cancelled.
    ///
    /// # Errors
    ///
    /// See above.
    #[inline]
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Sets how many units of work the job has.
    pub fn set_total(&self, total: u64) {
        self.0.total.store(total, Ordering::Relaxed);
        self.0.done.store(0, Ordering::Relaxed);
    }

    /// Records `n` finished units.
    pub fn advance(&self, n: u64) {
        self.0.done.fetch_add(n, Ordering::Relaxed);
    }

    /// Progress as `(done, total)`.
    #[must_use]
    pub fn progress(&self) -> (u64, u64) {
        (
            self.0.done.load(Ordering::Relaxed),
            self.0.total.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let a = Cancel::new();
        let b = a.clone();
        assert!(b.check().is_ok());
        a.cancel();
        assert!(b.is_cancelled());
        assert!(matches!(b.check(), Err(Error::Cancelled)));
    }

    #[test]
    fn progress_counts() {
        let c = Cancel::new();
        c.set_total(10);
        c.advance(3);
        c.clone().advance(4);
        assert_eq!(c.progress(), (7, 10));
    }

    #[test]
    fn cancel_crosses_threads() {
        let c = Cancel::new();
        let worker = {
            let c = c.clone();
            std::thread::spawn(move || {
                let mut rows = 0u64;
                while c.check().is_ok() {
                    rows += 1;
                    std::thread::yield_now();
                }
                rows
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(5));
        c.cancel();
        assert!(worker.join().unwrap() > 0);
    }
}

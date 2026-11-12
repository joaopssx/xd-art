//! Row parallelism.
//!
//! Stages that compute each output row on its own go through [`rows`]. With the
//! `parallel` feature, rows are handed to rayon in groups worth at least [`MIN_WORK`]
//! units, so small jobs do not pay for scheduling. The caller says what a pixel costs:
//! a nearest sample reads one source pixel, a Lanczos sample 36, a box sample of a
//! 1920 to 160 reduction about 140. Without it, the same closure runs
//! on one thread. Each row is computed by the same code either way, which is why the
//! output is identical with any number of threads.

use crate::Result;
use crate::cancel::Cancel;

/// Smallest amount of work worth sending to another thread, in source pixels read.
pub const MIN_WORK: usize = 64 * 1024;

/// Calls `f(y, row)` for every row of `data` (rows are `width` items long). `cost` is
/// the work per item, in source pixels read. `init` creates per-thread scratch state,
/// passed to every call on that thread. Checks `cancel` before each group of rows.
///
/// # Errors
///
/// Returns [`crate::Error::Cancelled`] when the token is cancelled.
pub fn rows_with<T, S, I, F>(
    data: &mut [T],
    width: usize,
    cost: usize,
    cancel: &Cancel,
    init: I,
    f: F,
) -> Result<()>
where
    T: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, usize, &mut [T]) + Sync + Send,
{
    if width == 0 {
        return Ok(());
    }
    let per_task = (MIN_WORK / (width * cost.max(1))).max(1);
    let group = |s: &mut S, (i, chunk): (usize, &mut [T])| -> Result<()> {
        cancel.check()?;
        for (j, row) in chunk.chunks_mut(width).enumerate() {
            f(s, i * per_task + j, row);
        }
        Ok(())
    };
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        data.par_chunks_mut(per_task * width)
            .enumerate()
            .try_for_each_init(init, group)
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut s = init();
        data.chunks_mut(per_task * width)
            .enumerate()
            .try_for_each(|c| group(&mut s, c))
    }
}

/// [`rows_with`] without scratch state.
///
/// # Errors
///
/// Returns [`crate::Error::Cancelled`] when the token is cancelled.
pub fn rows<T, F>(data: &mut [T], width: usize, cost: usize, cancel: &Cancel, f: F) -> Result<()>
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync + Send,
{
    rows_with(data, width, cost, cancel, || (), |(), y, row| f(y, row))
}

/// Runs `f` with rayon limited to `threads` threads, or directly without the
/// `parallel` feature. Tests use this to compare one thread against many.
///
/// # Panics
///
/// Panics if the thread pool cannot be created.
pub fn with_threads<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    #[cfg(feature = "parallel")]
    {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("thread pool")
            .install(f)
    }
    #[cfg(not(feature = "parallel"))]
    {
        let _ = threads;
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_once_in_order() {
        let mut data = vec![0usize; 300 * 200];
        rows(&mut data, 300, 1, &Cancel::new(), |y, row| row.fill(y + 1)).unwrap();
        for (y, row) in data.chunks(300).enumerate() {
            assert!(row.iter().all(|&v| v == y + 1));
        }
    }

    #[test]
    fn cancel_is_seen() {
        let c = Cancel::new();
        c.cancel();
        let mut data = vec![0u8; 100];
        assert!(rows(&mut data, 10, 1, &c, |_, _| {}).is_err());
    }

    #[test]
    fn scratch_is_per_thread() {
        let mut data = vec![0u32; 1000 * 64];
        rows_with(
            &mut data,
            1000,
            1,
            &Cancel::new(),
            Vec::<u32>::new,
            |s, y, row| {
                s.push(y as u32);
                row.fill(y as u32);
            },
        )
        .unwrap();
        assert_eq!(data[999 * 64 / 1000 * 1000], (999 * 64 / 1000) as u32);
    }
}

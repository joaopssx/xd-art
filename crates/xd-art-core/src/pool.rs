//! Reusable buffers.
//!
//! Moving a slider reruns a few stages on images of the same size again and again.
//! Instead of allocating fresh buffers each time, stages take them from a [`Pool`] and
//! the pipeline hands old results back when it lets go of them. After the first run, a
//! rerun at the same size gets every buffer from the pool.

use std::sync::Mutex;

use crate::buffer::{Indexed, Px, Work};

/// How many buffers of each kind the pool holds on to.
const KEEP: usize = 8;

/// A thread safe stash of buffers.
#[derive(Debug, Default)]
pub struct Pool {
    px: Mutex<Vec<Vec<Px>>>,
    bytes: Mutex<Vec<Vec<u8>>>,
}

fn take<T>(list: &Mutex<Vec<Vec<T>>>, len: usize) -> Vec<T> {
    let mut list = list
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Smallest buffer that fits; otherwise the largest, which grows the least.
    let fit = list
        .iter()
        .enumerate()
        .filter(|(_, v)| v.capacity() >= len)
        .min_by_key(|(_, v)| v.capacity())
        .or_else(|| list.iter().enumerate().max_by_key(|(_, v)| v.capacity()))
        .map(|(i, _)| i);
    let mut v = fit.map(|i| list.swap_remove(i)).unwrap_or_default();
    v.clear();
    v
}

fn give<T>(list: &Mutex<Vec<Vec<T>>>, v: Vec<T>) {
    if v.capacity() == 0 {
        return;
    }
    let mut list = list
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if list.len() < KEEP {
        list.push(v);
    } else if let Some((i, _)) = list
        .iter()
        .enumerate()
        .min_by_key(|(_, b)| b.capacity())
        .filter(|(_, b)| b.capacity() < v.capacity())
    {
        list[i] = v;
    }
}

impl Pool {
    /// An empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A working image of the given size, filled with `fill`.
    #[must_use]
    pub fn work(&self, width: u32, height: u32, fill: Px) -> Work {
        let len = width as usize * height as usize;
        let mut px = take(&self.px, len);
        px.resize(len, fill);
        Work { width, height, px }
    }

    /// A working image with the same size and pixels as `src`.
    #[must_use]
    pub fn copy(&self, src: &Work) -> Work {
        let mut px = take(&self.px, src.px.len());
        px.extend_from_slice(&src.px);
        Work {
            width: src.width,
            height: src.height,
            px,
        }
    }

    /// An empty pixel vector with room for `len` pixels when the pool has one.
    #[must_use]
    pub fn px(&self, len: usize) -> Vec<Px> {
        let mut v = take(&self.px, len);
        v.reserve(len);
        v
    }

    /// A byte vector of length `len`, filled with `fill`.
    #[must_use]
    pub fn bytes(&self, len: usize, fill: u8) -> Vec<u8> {
        let mut v = take(&self.bytes, len);
        v.resize(len, fill);
        v
    }

    /// An opaque indexed image filled with index 0.
    #[must_use]
    pub fn indexed(&self, width: u32, height: u32) -> Indexed {
        Indexed {
            width,
            height,
            index: self.bytes(width as usize * height as usize, 0),
            clear: Vec::new(),
        }
    }

    /// Gives a working image back.
    pub fn recycle(&self, w: Work) {
        give(&self.px, w.px);
    }

    /// Gives a pixel vector back.
    pub fn recycle_px(&self, v: Vec<Px>) {
        give(&self.px, v);
    }

    /// Gives a byte vector back.
    pub fn recycle_bytes(&self, v: Vec<u8>) {
        give(&self.bytes, v);
    }

    /// Gives an indexed image back.
    pub fn recycle_indexed(&self, i: Indexed) {
        give(&self.bytes, i.index);
    }

    /// Number of buffers currently held, for tests and the debug overlay.
    #[must_use]
    pub fn held(&self) -> usize {
        let lock = |m: &Mutex<Vec<Vec<Px>>>| {
            m.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        };
        lock(&self.px)
            + self
                .bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_come_back() {
        let pool = Pool::new();
        let a = pool.work(16, 16, [1; 4]);
        let ptr = a.px.as_ptr();
        pool.recycle(a);
        assert_eq!(pool.held(), 1);
        let b = pool.work(8, 8, [2; 4]);
        assert_eq!(b.px.as_ptr(), ptr);
        assert!(b.px.iter().all(|p| *p == [2; 4]));
        assert_eq!(b.px.len(), 64);
    }

    #[test]
    fn picks_the_smallest_fit() {
        let pool = Pool::new();
        pool.recycle_bytes(Vec::with_capacity(1000));
        pool.recycle_bytes(Vec::with_capacity(10));
        pool.recycle_bytes(Vec::with_capacity(100));
        assert!(pool.bytes(50, 0).capacity() >= 100);
        assert!(pool.bytes(50, 0).capacity() >= 1000);
        assert_eq!(pool.bytes(5, 7), vec![7; 5]);
    }

    #[test]
    fn keeps_a_bounded_number() {
        let pool = Pool::new();
        for i in 1..20 {
            pool.recycle_bytes(Vec::with_capacity(i));
        }
        assert_eq!(pool.held(), KEEP);
        pool.recycle_bytes(Vec::new());
        assert_eq!(pool.held(), KEEP);
    }

    #[test]
    fn copy_matches_source() {
        let pool = Pool::new();
        let mut w = pool.work(3, 2, [0; 4]);
        w.px[4] = [5, 6, 7, 8];
        assert_eq!(pool.copy(&w), w);
    }
}

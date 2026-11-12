//! Counts heap allocations to check that reruns reuse buffers.
//!
//! This is its own test binary because it replaces the global allocator. The counter is
//! per thread so the test harness running other tests at the same time does not matter.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use xd_art_core::buffer::Image;
use xd_art_core::pool::Pool;

struct Counting;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator unchanged and only bumps a thread local
// counter, which does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations<F: FnOnce()>(f: F) -> usize {
    let before = COUNT.with(Cell::get);
    f();
    COUNT.with(Cell::get) - before
}

#[test]
fn pool_reuse_does_not_allocate() {
    let pool = Pool::new();
    let warm = pool.work(64, 64, [0; 4]);
    pool.recycle(warm);
    let n = allocations(|| {
        for _ in 0..10 {
            let w = pool.work(64, 64, [1; 4]);
            let c = pool.copy(&w);
            pool.recycle(w);
            pool.recycle(c);
        }
    });
    // The first copy needs a second buffer; after that both live in the pool.
    assert!(n <= 1, "{n} allocations");
}

#[test]
fn converting_into_a_recycled_buffer_does_not_allocate() {
    let img = Image::new(32, 32, [1, 2, 3]).unwrap();
    let pool = Pool::new();
    let mut w = pool.work(32, 32, [0; 4]);
    let n = allocations(|| {
        for _ in 0..10 {
            img.write_work(&mut w);
        }
    });
    assert_eq!(n, 0);
}

//! The counting allocator as a process's global allocator, as the `prism` binary declares it
//! (D7 of `reactive-app-verification`): allocating on a thread that never touched its counts
//! does not recurse into the allocator, and the counts follow what the thread allocates.

use prism_runtime::allocation_counter::{enable, thread_counts, CountingAllocator};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn a_fresh_thread_allocates_through_the_counting_allocator() {
    enable();
    let counts = std::thread::spawn(|| {
        // The thread's first allocation is also its first touch of the counts.
        let before = thread_counts();
        let boxed = std::hint::black_box(Box::new([0u8; 64]));
        let mut grown = std::hint::black_box(vec![1u8; 8]);
        grown.reserve(1024);
        drop(boxed);
        drop(grown);
        let after = thread_counts();
        (after.0 - before.0, after.1 - before.1)
    })
    .join()
    .unwrap();
    // The box, the vector and its growth (one of each), then the two frees.
    assert_eq!(counts, (3, 3));
}

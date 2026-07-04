//! Tile worker pool: control block, dispatch/join protocol, worker entry.
//!
//! Mechanics in `docs/design.md` (Threads). The control block is the only
//! shared-mutable static. Job slots are written by the coordinator strictly
//! before the Release epoch bump and read by workers strictly after their
//! Acquire epoch load; results travel back over the Release `remaining`
//! decrement / Acquire join load. All cross-thread data rides those two
//! edges — no per-field atomics.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

pub(crate) const WORKER_COUNT: usize = 3;

/// One dispatched unit of work. Placeholder fill-job shape until
/// tile-parallel decode lands: fill `len` bytes at `addr` with a
/// position-dependent pattern from `seed`.
#[derive(Clone, Copy)]
pub(crate) struct Job {
    pub addr: usize,
    pub len: usize,
    pub seed: u8,
}

struct Slot {
    job: UnsafeCell<Option<Job>>,
}

struct ControlBlock {
    /// Bumped once per dispatched wave; workers park on it between waves.
    /// Workers treat any change from the last value they acted on as "read
    /// your slot", so a worker that spawns after a dispatch still sees it.
    epoch: AtomicU32,
    /// Workers that have not yet acknowledged the current wave; always
    /// starts at WORKER_COUNT. Every worker decrements once per wave — job
    /// or not — after it is done reading its slot, so a completed join
    /// also means no worker can still be looking at a slot while the
    /// coordinator rewrites them for the next wave. (Counting only
    /// assigned jobs would let join return while an idle worker is still
    /// on its way to read a None slot the next dispatch is overwriting.)
    /// The worker reaching zero notifies the joining coordinator.
    remaining: AtomicU32,
    slots: [Slot; WORKER_COUNT],
}

// Safety: slot access is ordered by the epoch/remaining protocol — the
// coordinator writes slots only between an observed join (remaining == 0)
// and the next epoch bump; worker N reads only slot N, only after observing
// a new epoch.
unsafe impl Sync for ControlBlock {}

static POOL: ControlBlock = ControlBlock {
    epoch: AtomicU32::new(0),
    remaining: AtomicU32::new(0),
    slots: [const { Slot { job: UnsafeCell::new(None) } }; WORKER_COUNT],
};

const WAIT_TIMED_OUT: i32 = 2;

/// Publish a wave: one optional job per worker. Requires the previous wave
/// to be joined. Every slot is rewritten, so a stale job can never re-run.
pub(crate) fn dispatch(jobs: &[Option<Job>; WORKER_COUNT]) {
    assert_eq!(
        POOL.remaining.load(Ordering::Relaxed),
        0,
        "dispatch before previous wave joined"
    );
    for (slot, job) in POOL.slots.iter().zip(jobs) {
        unsafe { *slot.job.get() = *job };
    }
    POOL.remaining
        .store(WORKER_COUNT as u32, Ordering::Relaxed);
    POOL.epoch.fetch_add(1, Ordering::Release);
    unsafe {
        core::arch::wasm32::memory_atomic_notify(POOL.epoch.as_ptr().cast(), u32::MAX);
    }
}

/// Block until every worker has acknowledged the current wave (all jobs
/// done, no worker still reading a slot). `timeout_ns` bounds each
/// individual wait, not the total; negative waits forever. Returns false
/// on timeout.
pub(crate) fn join(timeout_ns: i64) -> bool {
    loop {
        let remaining = POOL.remaining.load(Ordering::Acquire);
        if remaining == 0 {
            return true;
        }
        let waited = unsafe {
            core::arch::wasm32::memory_atomic_wait32(
                POOL.remaining.as_ptr().cast(),
                remaining as i32,
                timeout_ns,
            )
        };
        if waited == WAIT_TIMED_OUT {
            return POOL.remaining.load(Ordering::Acquire) == 0;
        }
    }
}

/// Worker thread entry, reached via `vip9r_worker_main` on a fresh instance
/// whose shadow stack JS already rebound. Parks on the epoch, runs this
/// worker's slot when it changes. Never returns; teardown is JS
/// `Worker.terminate()`, which is safe mid-wait (probed on d8).
pub(crate) fn worker_main(worker_index: u32) -> ! {
    let index = worker_index as usize;
    assert!(index < WORKER_COUNT, "worker index out of range");
    let slot = &POOL.slots[index];
    let mut seen = 0;
    loop {
        let current = POOL.epoch.load(Ordering::Acquire);
        if current == seen {
            unsafe {
                core::arch::wasm32::memory_atomic_wait32(
                    POOL.epoch.as_ptr().cast(),
                    current as i32,
                    -1,
                );
            }
            continue;
        }
        seen = current;
        if let Some(job) = unsafe { *slot.job.get() } {
            run_job(job);
        }
        if POOL.remaining.fetch_sub(1, Ordering::Release) == 1 {
            unsafe {
                core::arch::wasm32::memory_atomic_notify(POOL.remaining.as_ptr().cast(), 1);
            }
        }
    }
}

// Placeholder job body until tile-parallel decode lands.
fn run_job(job: Job) {
    let bytes = unsafe { core::slice::from_raw_parts_mut(job.addr as *mut u8, job.len) };
    for (offset, byte) in bytes.iter_mut().enumerate() {
        *byte = job.seed.wrapping_add(offset as u8);
    }
}

// The d8 test runner spawns live workers for tests under this module path
// (matched on "::pool::" in the export name); without them a dispatch would
// never join, which the bounded join turns into a failure instead of a hang.
#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{Job, WORKER_COUNT, dispatch, join};

    const JOIN_TIMEOUT_NS: i64 = 5_000_000_000;
    const JOB_LEN: usize = 64;

    // Job buffers live on the coordinator stack on purpose: the stack region
    // is ordinary shared memory and workers must be able to write anywhere
    // the coordinator can point them.
    fn job(buffer: &mut [u8; JOB_LEN], seed: u8) -> Job {
        buffer.fill(0);
        Job {
            addr: buffer.as_mut_ptr() as usize,
            len: JOB_LEN,
            seed,
        }
    }

    fn assert_filled(buffer: &[u8; JOB_LEN], seed: u8) {
        for (offset, byte) in buffer.iter().enumerate() {
            assert_eq!(
                *byte,
                seed.wrapping_add(offset as u8),
                "offset {offset} seed {seed}"
            );
        }
    }

    #[test]
    fn full_waves_dispatch_and_join() {
        let mut buffers = [[0u8; JOB_LEN]; WORKER_COUNT];
        for round in 0..50u8 {
            let [b0, b1, b2] = &mut buffers;
            let jobs = [
                Some(job(b0, round)),
                Some(job(b1, round.wrapping_add(1))),
                Some(job(b2, round.wrapping_add(2))),
            ];
            dispatch(&jobs);
            // Coordinator-side work between dispatch and join, like the
            // mop-up tile in the real frame loop.
            let mut own = [0u8; JOB_LEN];
            for (offset, byte) in own.iter_mut().enumerate() {
                *byte = round.wrapping_add(offset as u8);
            }
            assert!(join(JOIN_TIMEOUT_NS), "join timed out (round {round})");
            assert_filled(&buffers[0], round);
            assert_filled(&buffers[1], round.wrapping_add(1));
            assert_filled(&buffers[2], round.wrapping_add(2));
            assert_filled(&own, round);
        }
    }

    #[test]
    fn partial_wave_completes_with_idle_workers() {
        let mut buffer = [0u8; JOB_LEN];
        let jobs = [None, Some(job(&mut buffer, 0x40)), None];
        dispatch(&jobs);
        assert!(join(JOIN_TIMEOUT_NS), "partial wave join timed out");
        assert_filled(&buffer, 0x40);

        // The idle workers must still be parked and responsive.
        let mut buffers = [[0u8; JOB_LEN]; WORKER_COUNT];
        let [b0, b1, b2] = &mut buffers;
        let jobs = [Some(job(b0, 1)), Some(job(b1, 2)), Some(job(b2, 3))];
        dispatch(&jobs);
        assert!(join(JOIN_TIMEOUT_NS), "follow-up wave join timed out");
        assert_filled(&buffers[0], 1);
        assert_filled(&buffers[1], 2);
        assert_filled(&buffers[2], 3);
    }
}

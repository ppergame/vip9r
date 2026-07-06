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

/// One dispatched unit of work. Tile jobs carry plain Copy data: a pointer
/// to the wave's coordinator-stack shared context plus this band's column
/// range and destination pointers. Ownership of everything they target is
/// handed to the worker by the epoch Release and returned by the remaining
/// Release decrement.  The wasm-test fill variant is kept only for protocol
/// smoke tests under the `::pool::` export name.
#[derive(Clone, Copy)]
pub(crate) enum Job {
    Tile(crate::tile_syntax::TileJob),
    FusedTileFilter(crate::tile_syntax::FusedTileFilterJob),
    LoopFilter(crate::tile_syntax::LoopFilterJob),
    #[cfg(feature = "wasm-tests")]
    Fill(FillJob),
}

#[cfg(feature = "wasm-tests")]
#[derive(Clone, Copy)]
pub(crate) struct FillJob {
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
    slots: [const {
        Slot {
            job: UnsafeCell::new(None),
        }
    }; WORKER_COUNT],
};

/// Whether the frontend has spawned the worker pool over this memory. The
/// pool is all-or-nothing: join counts acknowledgements from every worker,
/// so dispatching with fewer than WORKER_COUNT live workers hangs. Frontends
/// that spawn no workers (wasm unit tests outside this module, harnesses
/// without a pool flag) leave this off and decode stays serial.
static ACTIVE: AtomicU32 = AtomicU32::new(0);

/// Called by the frontend (via `vip9r_pool_activate`) after all
/// WORKER_COUNT workers are spawned. Workers that are still starting up are
/// fine: a worker that first loads `epoch` after a dispatch sees the bumped
/// value and reads its slot.
pub(crate) fn activate() {
    ACTIVE.store(1, Ordering::Relaxed);
}

pub(crate) fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed) != 0
}

const WAIT_TIMED_OUT: i32 = 2;

/// Publish a wave: one optional job per worker. Requires the previous wave
/// to be joined. Every slot is rewritten, so a stale job can never re-run.
pub(crate) fn dispatch(jobs: &[Option<Job>; WORKER_COUNT]) {
    assert!(is_active(), "dispatch without an activated worker pool");
    assert_eq!(
        POOL.remaining.load(Ordering::Relaxed),
        0,
        "dispatch before previous wave joined"
    );
    for (slot, job) in POOL.slots.iter().zip(jobs) {
        unsafe { *slot.job.get() = *job };
    }
    POOL.remaining.store(WORKER_COUNT as u32, Ordering::Relaxed);
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

/// Monotone progress word with a wanted-mark, for wavefront dependencies
/// with a *single concurrent waiter* — the filter row watermarks, where only
/// the claimant of row r+1 ever waits on row r. `value` is the published
/// progress; `wanted` is the waiter's advertised target, so the producer
/// notifies exactly when a store crosses it instead of on every superblock.
/// Both reset by constructing a fresh `Watermark` per frame/wave.
///
/// Multi-waiter watermarks (the per-band decode row counters, waited on by
/// several gated filter rows at once) must NOT use this type: a single
/// wanted word coalesces concurrent targets to their max, so an early-row
/// waiter would oversleep until the band reached the farthest requested row
/// — measured as a phase-dependent multi-percent regression. They stay on
/// the plain always-notify helpers below, which is cheap at their once-per-
/// SB-row publish cadence.
pub(crate) struct Watermark {
    value: AtomicU32,
    wanted: AtomicU32,
}

impl Watermark {
    pub(crate) const fn new(value: u32) -> Self {
        Self {
            value: AtomicU32::new(value),
            wanted: AtomicU32::new(0),
        }
    }

    /// Block until the watermark reaches at least `target`.
    ///
    /// Lost-wakeup avoidance relies on the wasm memory model: wasm atomic
    /// memory operations, including wait's atomic compare, are sequentially
    /// consistent. The Rust operations request `SeqCst` explicitly so the
    /// source-level proof matches the wasm codegen.
    ///
    /// The waiter publishes its target with `fetch_max`, then reloads the
    /// progress word before it can sleep. If the producer stored a
    /// satisfying value and checked `wanted` before the `fetch_max`, the
    /// reload is ordered after that store and observes the new progress. If
    /// the reload still sees old progress, the `fetch_max` is ordered before
    /// the producer's later `wanted` load, so the store that crosses the
    /// target will notify. If the store races between the reload and
    /// `memory_atomic_wait32`, wait32's atomic compare sees the changed
    /// value and returns without parking. The bad interleaving "load old,
    /// producer stores and sees no wanted, then publish wanted and sleep
    /// forever" is excluded because the wanted mark is published before any
    /// load whose value can feed wait32.
    ///
    /// The satisfied fast path returns on a plain load without touching
    /// `wanted`: in wavefront steady state most checks are already met, and
    /// skipping the read-modify-write avoids bouncing the watermark's cache
    /// line between producer and waiter. Its load never feeds wait32, so the
    /// publish-before-sleep ordering above is untouched.
    pub(crate) fn wait_at_least(&self, target: u32) {
        if self.value.load(Ordering::SeqCst) >= target {
            return;
        }
        self.wanted.fetch_max(target, Ordering::SeqCst);
        loop {
            let current = self.value.load(Ordering::SeqCst);
            if current >= target {
                return;
            }
            unsafe {
                core::arch::wasm32::memory_atomic_wait32(
                    self.value.as_ptr().cast(),
                    current as i32,
                    -1,
                );
            }
        }
    }

    /// Publish a new value and wake the waiter only when this store crosses
    /// its advertised target (`old < wanted <= new` — the crossing test, so
    /// a stale wanted mark left by a waiter that never slept does not turn
    /// every later store into a futex wake). The `SeqCst` store carries the
    /// filtered pixels published before it to the waiter's `SeqCst` load.
    pub(crate) fn store(&self, value: u32) {
        let old = self.value.load(Ordering::SeqCst);
        debug_assert!(old <= value, "watermarks must be monotone within a wave");
        self.value.store(value, Ordering::SeqCst);
        let wanted = self.wanted.load(Ordering::SeqCst);
        if old < wanted && value >= wanted {
            unsafe {
                core::arch::wasm32::memory_atomic_notify(self.value.as_ptr().cast(), u32::MAX);
            }
        }
    }

    /// Publish the row's terminal value and wake unconditionally. Row-end
    /// and abandon paths use this: every valid target is at most this value,
    /// so after the notify the waiter observes completion.
    pub(crate) fn store_final(&self, value: u32) {
        self.value.store(value, Ordering::SeqCst);
        unsafe {
            core::arch::wasm32::memory_atomic_notify(self.value.as_ptr().cast(), u32::MAX);
        }
    }
}

/// Block until `watermark` reaches at least `target`. Progress values are
/// monotone within a wave, so a stale load only causes an extra wait
/// iteration; the wait itself is race-free because wait32 rechecks the
/// expected value atomically. For multi-waiter watermarks (decode band
/// gates), paired with the always-notify `watermark_store`.
pub(crate) fn watermark_wait_at_least(watermark: &AtomicU32, target: u32) {
    loop {
        let current = watermark.load(Ordering::Acquire);
        if current >= target {
            return;
        }
        unsafe {
            core::arch::wasm32::memory_atomic_wait32(watermark.as_ptr().cast(), current as i32, -1);
        }
    }
}

/// Publish a new watermark value and wake every waiter. The Release store
/// carries the decoded rows published before it; waiters pair with the
/// Acquire load in `watermark_wait_at_least`.
pub(crate) fn watermark_store(watermark: &AtomicU32, value: u32) {
    watermark.store(value, Ordering::Release);
    unsafe {
        core::arch::wasm32::memory_atomic_notify(watermark.as_ptr().cast(), u32::MAX);
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

fn run_job(job: Job) {
    match job {
        Job::Tile(job) => crate::tile_syntax::run_tile_job(job),
        Job::FusedTileFilter(job) => crate::tile_syntax::run_fused_tile_filter_job(job),
        Job::LoopFilter(job) => crate::tile_syntax::run_loop_filter_job(job),
        #[cfg(feature = "wasm-tests")]
        Job::Fill(job) => run_fill_job(job),
    }
}

#[cfg(feature = "wasm-tests")]
fn run_fill_job(job: FillJob) {
    // SAFETY: Pool smoke tests pass stack buffers that remain live until
    // after join.  Slot handoff and completion use the same Release/Acquire
    // edges as real tile jobs.
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
    use super::{FillJob, Job, WORKER_COUNT, dispatch, join};

    const JOIN_TIMEOUT_NS: i64 = 5_000_000_000;
    const JOB_LEN: usize = 64;

    // Job buffers live on the coordinator stack on purpose: the stack region
    // is ordinary shared memory and workers must be able to write anywhere
    // the coordinator can point them.
    fn job(buffer: &mut [u8; JOB_LEN], seed: u8) -> Job {
        buffer.fill(0);
        Job::Fill(FillJob {
            addr: buffer.as_mut_ptr() as usize,
            len: JOB_LEN,
            seed,
        })
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

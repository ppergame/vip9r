# vip9r performance ratchet

Contract for the first optimization loop. This is intentionally small: enough
for a new session to build the harness and avoid known traps.

## Purpose

The ratchet gates optimization work on two facts:

- the selected clip/window still passes md5 validation
- the candidate is faster on the target path

Goldens remain the correctness authority. Ratchet numbers are optimization
evidence only when produced by the normal harness from a named repo revision;
ad-hoc wasm files are exploration.

## Full-decode benchmark

Use the release wasm build. Disable release overflow checks before treating
numbers as representative; then run the strict golden corpus once.

`wasm-golden --bench --bench-frames START:LAST` reuses the golden frontend
shape:

1. parse clip and sidecar md5
2. compile and instantiate wasm
3. decode the selected output-frame window once with md5 validation
4. warm up by decoding the same window without md5
5. measure repeated decodes of the same window without md5 until target duration
6. report frames, outputs, elapsed time, ms/frame, and fps

Defaults:

- clip: `/bulk/vip9r/chromium/bear-vp9.ivf`
- frames: `0:81`
- warmup: about `1s`
- measurement target: about `5s`
- d8 tiering: hardcode top-tier Wasm, initially `--no-liftoff`

`START:LAST` uses zero-based md5 visible-frame indices, inclusive. `START`
selects the first md5 sidecar line to measure and `LAST` selects the last.
Nonzero starts require WebM keyframe metadata; the selected visible
packet must be marked keyframe, and timed decode starts at that packet. IVF only
supports start `0`.

Do not use raw correctness-mode `wasm-golden` wall time as the ratchet result.
Its per-frame plane copy and md5 work are validation overhead, not decoder
timing.

## Device state and queue

Use a persistent device root:

```text
/data/local/tmp/vip9r/
  bin/      # d8, ffmpeg, V8 sidecar data files
  media/    # persistent corpus subset
  runs/     # per-run bench.js, vip9r.wasm, result.json
```

Assume installed binaries and copied media remain stable while grinder jobs run.
Dedicated sync commands can copy `bin/` and `media/`; each benchmark run should
copy only its JS driver and wasm into a fresh run directory.

Multiple grinders may submit device work concurrently, but device access is
serialized by a small daemon. The submit interface is blocking:

```sh
vip9r-perf-submit candidate.wasm
```

The command returns result JSON when the queued run finishes. No async status API
is needed for v0.

Host benchmarking is a cheap reject filter. Device benchmarking is what counts
for performance decisions.

## Profiling

Profiling is attribution, not the ratchet. Use it to choose grinder tasks and
microbenchmark slots; confirm wins with full-decode timing.

Working Android path:

- run d8 top-tier Wasm
- pass `--perf-basic-prof --perf-basic-prof-path=/data/local/tmp`
- sample with `simpleperf record`
- report with `simpleperf report --sort dso,symbol`

`--perf-basic-prof` emits `perf-<pid>.map`, which `simpleperf` uses to map JIT
PCs to vip9r Wasm function names. `--perf-prof`/jitdump is not useful with the
current V8 pins for vip9r user Wasm symbols. `--prof` tick logs are only useful
when paired with perf maps.

The first Android probes agreed on the broad hotspot order:
`inter_predict_sample`, `loop_filter_frame`, `decode_block`, inter prediction
dispatch, inverse DCT, and residual butterfly helpers. Treat that as a starting
hypothesis; the probes used `wasm-golden`, so validation overhead can appear.

Concrete d8/simpleperf mechanics live in [`docs/d8.md`](d8.md).

## Microbench slots

Microbenchmarks are transient grinder tools for suspected hot slices. A grinder
may add an ad-hoc export:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn vip9r_bench_run(slot: u32, inner_iters: u32) -> u32
```

`vip9r-perf-submit --slot N` runs that export instead of the full-decode loop.
The slot is a task-owned linear list, not a stable Wasm function index. The task
or final report must say what each slot means.

JS owns warmup duration, target duration, and timed batches. Wasm owns the tight
inner loop so tiny operations do not measure JS-to-Wasm call overhead. The
return value is a sink.

No v0 setup call or fixture protocol. A transient slot may lazily initialize
data, reuse state left by validation decode, or hardcode a fixture. Standardize
only after repeated tasks need the same shape.

Microbench wins only count after full-decode timing confirms wall-time
improvement.

## Non-goals for v0

- content-addressed stores
- async job/status APIs
- exposed V8 tier selection
- permanent microbenchmark APIs
- browser playback measurement

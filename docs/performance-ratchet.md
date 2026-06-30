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
3. warm up by decoding the selected output-frame window; the first warmup pass
   also performs md5 validation
4. continue warmup without md5 if the first pass did not reach the warmup target
5. measure repeated decodes of the same window without md5 until target duration
6. report frames, outputs, elapsed time, ms/frame, and fps

Defaults:

- clip: `/bulk/vip9r/chromium/bear-vp9.ivf`
- frames: `0:81`
- warmup: about `1s`
- measurement target: about `5s`
- d8 tiering: hardcode top-tier Wasm, initially `--no-liftoff`

The measurement target is also the maximum admissible time for one complete
decode pass through the selected window. Timed warmup and measurement both fail
if a pass exceeds that limit before producing the selected output frames.
Measurement may still overshoot the phase target by finishing a final whole
pass; bounded complete passes are preferable to partial-window timing.
Benchmark validation is part of the first timed warmup pass, so an oversized
window fails there instead of spending unbounded time on a standalone validation
decode.

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

Device setup is external to the daemon: `bin/` must contain the V8 payload, and
`media/` must contain the needed corpus files plus `.md5` sidecars. Assume those
files remain stable while grinder jobs run. Each benchmark run should copy only
its JS driver bundle and wasm into a fresh run directory.

`vip9r-perf-daemon --serial SERIAL` runs Android `d8` from the persistent
`bin/` directory and refuses to start if `d8`, `icudtl.dat`, or
`snapshot_blob.bin` are missing. Benchmark requests verify that the requested
media file and its `.md5` sidecar already exist under persistent `media/`;
daemon runs do not copy media or V8 payloads. The daemon may cache its stable
baseline wasm under `runs/_cache/`, while each request copies only the small JS
driver bundle and candidate wasm into a fresh `runs/` directory.

Device CPU affinity is a daemon/request-level selection, not a thermal,
frequency, or topology policy. Use `--pin any|all|cpu:N|mask:HEX` on the daemon
for a default, or on `vip9r-perf-submit` for a per-job override. Non-`any` pins
resolve to an explicit CPU mask. The daemon verifies the effective
`Cpus_allowed_list` before running d8 and reports the requested mask, requested
CPU list, and verified CPU list in result JSON.

Multiple grinders may submit device work concurrently, but device access is
serialized by a small daemon. The submit interface is blocking:

```sh
vip9r-perf-submit candidate.wasm --media realworld/clip.webm
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

`wasm-microbench --slot N` runs that export directly. `vip9r-perf-submit
candidate.wasm --slot N` submits the same candidate-only run through the daemon.
The slot is a task-owned linear list, not a stable Wasm function index. The task
or final report must say what each slot means.

JS owns warmup duration, target duration, and timed batches. Wasm owns the tight
inner loop so tiny operations do not measure JS-to-Wasm call overhead. The
return value is a sink.

Microbench JSON is a single stdout object. Unlike full-decode benchmarking,
daemon microbenchmarks do not run the baseline wasm. They are attribution tools,
not ratchet evidence.

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

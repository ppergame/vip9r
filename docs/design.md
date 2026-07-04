# vip9r — design

Working notes on constraints and mechanics. requirements.md holds the project
framing, roles, and durable decisions; this file should stay small enough to
guide the next session without pretending the decoder architecture is settled.

## Stable constraints

- Target is a freestanding `wasm32-unknown-unknown` module with manual bindings,
  a small export surface, and streaming I/O at frame granularity.
- Decoder work is Rust-first and spec-first. libvpx is a correctness/debugging
  oracle, not an implementation template.
- Unsafe Rust is allowed where integration or measured performance requires it;
  safe Rust remains the default.
- The terminal sinks are d8 frame-md5 validation and browser `VideoFrame`
  presentation. There is no encode path.
- SIMD, relaxed SIMD, workers, allocator shape, and internal boundaries are
  choices to earn with evidence, not decisions to pre-bake into the docs.
  - 2026-07: wasm simd128 is in scope for M3. It is baseline in shipped Chrome
    and V8 lowers it to NEON on both device targets (arm64 and the streamer's
    arm32), so it is portable, not CPU-specific tuning. Relaxed SIMD remains
    stretch; threads are scoped as M6 (see the Threads section).

## Correctness

Bit-exact per-frame output against libvpx md5 is the eventual bar, reached in
two phases rather than gated incrementally:

- **Code-complete first (M1).** Build the whole profile 0 / 8-bit decode path
  from the spec. Almost nothing produces a correct full frame until the entire
  pipeline (entropy → dequant → inverse transform → prediction → reconstruction
  → loop filter) exists, so per-frame goldens cannot gate early work. M1
  verification was builds, `clippy`, targeted unit tests, engineering judgment,
  and a frame-md5 harness running end to end with output allowed to be wrong.
- **Bring-up second (M2).** Drive frame-md5 correctness green. The fast host
  harness has been retired; the d8 wasm driver is the canonical correctness loop
  and shipping-path parity check. The first frame whose md5 matches is the
  first-bit-exact milestone; from there it is debugging.

libvpx-generated outputs are useful when the spec or test vectors are not
enough. Intermediate checks may be added when they make failures easier to
localize, but the tap points and granularity should follow the implementation we
actually have. Do not commit the project to libvpx component boundaries just
because they are available to instrument.

Spec note: the local VP9 v0.7 draft's partition probability prose appears to
reverse the `FrameIsIntra` condition. The decoder uses the fixed
`kf_partition_probs` table for key/intra frame partition syntax, matching the
table naming and key-frame syntax context.

Segmentation note: current-frame block metadata and the persistent segmentation
map are related but distinct. `segment_id` feeds current-frame syntax,
dequantization, and loop filtering. The saved `PrevSegmentIds` map only changes
when the frame updates the segmentation map, and otherwise persists across
segmentation-disabled or `update_map == false` frames unless reset by past
independence.

### Golden runner

The golden runner decodes a vector frame by frame and compares per-frame md5
against the `.md5` golden. The libvpx md5 protocol has sharp edges:

- The hash is over the raw I420 frame: Y (visible `d_w`×`d_h`), then U, then V
  at chroma dims (`⌈w/2⌉`×`⌈h/2⌉`), visible dimensions only — no stride padding.
- Only _shown_ frames produce a line, in display order. A VP9 superframe packs
  several coded frames into one demuxed packet but usually shows one;
  `show_existing_frame` re-emits a stored frame and gets its own line. So output
  frame count ≤ coded frame count.
- Golden format is one line per shown frame: `<md5hex>  <name>.i420`. Compare
  positionally.
- Some SVC IVF vectors advertise `0x0` container dimensions and carry a
  dimensioned md5 sidecar for the top spatial layer only. The runner may derive
  decoder limits from those sidecar names; for zero-dimension IVF only, decoded
  outputs whose decoded size is absent from the sidecar dimensions are skipped
  before positional comparison. Do not apply this skip rule to ordinary IVF/WebM
  vectors: md5 filename dimensions are not reliable enough for a global filter.

The d8 wasm driver is the canonical golden path and exercises the manual
boundary used by the shipping path. In the main checkout, use the short command:

```sh
wasm-golden
```

`wasm-golden` builds the release wasm module and runs d8 against prebuilt JS
runner artifacts. It defaults to `/bulk/vip9r/chromium/bear-vp9.ivf`, and the
driver defaults the golden path to the `.md5` sidecar. The optional input path
may be IVF or WebM. Normal stdout is one compact JSON object with `mode`,
`ok`, workload metadata, frame counts, and the first few mismatches. Diagnostics
and progress go to stderr so daemon/device runs can persist stdout as
`result.json`. The runner exits non-zero on decode errors, missing/extra shown
frames, or md5 mismatches; `wasm-golden --allow-mismatch` only permits wrong
frame hashes for code-complete smoke runs. Grinder sandboxes do not carry these
JS runner artifacts; use `vip9r-perf-submit validate [--allow-mismatch]`
instead. The current wasm driver parses all 82 coded frames in
`bear-vp9.ivf`, emits 82 shown frames, applies the in-loop filter, and strict
md5 passes with 82 matched frames and no mismatches/missing/extra frames.
Long local corpus runs can use `wasm-golden --progress-frames=N` to print
periodic compared-frame progress to stderr without changing the final pass/fail
criteria.

`scripts/vip9r-corpus-golden.py` runs strict golden validation across the whole
md5-backed `/bulk/vip9r` corpus on host d8, one process per core. The default
compliance set excludes a hardcoded list of heavy movie/VOD clips and finishes
in about a minute on the workstation; it gates every merged optimization pass.
`--all` includes the heavy clips (hours of CPU, dominated by the
multi-thousand-frame movie clips) and runs in the background at pass
boundaries. When a deferred `--all` run fails, bisect by replaying only the
failing vectors across the candidate commits, not the corpus.

WebM demux is intentionally a narrow harness subset: one `V_VP9` video track is
selected, non-video tracks are ignored, `SimpleBlock` and `BlockGroup/Block`
payloads become VP9 packets, and laced VP9 blocks are rejected.
`Cues`/seeking/index interpretation is skipped by element size for now. The
parser is single-pass over a complete file-shaped input: it expects `Tracks`
before `Cluster`, does not follow `SeekHead`, and does not model split
init/media segments yet. For resize vectors whose `TrackEntry` dimensions are
smaller than later decoded frames, and for zero-dimension IVF vectors with
dimensioned sidecar names, the harness sizes the decoder from the maximum
dimensions encoded in the `.md5` sidecar frame names when available, falling
back to container dimensions.

Decode-path wasm failures are expected to be real core/no-std issues unless they
happen before `decode_next`, which still implicates the JS/wasm wrapper,
exported ABI, input copying, or packet setup. The JS runner reports packet,
timestamp, coded-frame, and output-frame context around decode calls.

All d8 frontends instantiate the wasm module with
`env.vip9r_log(kind, ptr, len)`. Rust `diag!` formats into a bounded stack
buffer and the JS sink copies bytes synchronously during the import call;
callers must not retain the raw wasm span. Routine golden and test runs print
nothing unless Rust emits diagnostics or panics. Log kind `0` is ordinary
diagnostics, `1` is wasm test failure output, and `2` is panic output.

### Decode API

The primary core API is packet splitting plus one-coded-frame decode. A demuxed
VP9 packet may be a superframe; `split_packet` returns up to 8 coded-frame byte
ranges. `Decoder::decode_coded_frame` consumes one range with an explicit
`DecodeWorkspace` and returns either no output or one shown frame. Packet-level
helpers are adapters, not the architecture.

`Decoder` owns VP9 semantic session state. Geometry-sized storage belongs to the
supplied workspace. A decoder session must keep using the same workspace memory;
previous-frame mode history for MV reference candidates lives there and is
addressed by decoder-owned slot metadata. `WorkspaceLayout` currently defines a
fixed arena with one current reconstruction frame slot plus 8 reference frame
slots, each using simple 4:2:0 byte capacity derived from instance max
dimensions, followed by two packed mode-history slots. Additional maps and
scratch should enter the layout only when implementation code actually consumes
them.

Shown-frame output borrows from the supplied workspace and is invalid after the
next decode or packet transition. Core output is an `I420Frame`: visible Y, U,
and V planes with a shared `PlaneShape` for width, height, and stride, plus a
backing byte slice. Compact I420 in libvpx-md5 order is a tools/harness
serialization, not the core output model.

The wasm boundary uses the same shape: one instance is one decoder session,
`begin_packet` stages packet ranges, and `decode_next` advances exactly one
coded frame. There is no reset API; JS recreates the instance on stream changes
or decode errors.

### Wasm boundary

The `vip9r` wasm module exposes the manual integer ABI used by the paired JS
binding: `vip9r_result_ptr`, `vip9r_required_pages`, `vip9r_init`,
`vip9r_reserve_input`, `vip9r_begin_packet`, and `vip9r_decode_next`. Mutating
exports return `0` for success or a negative error code; those codes are
binding details, not a stable external API.

`vip9r_required_pages(max_width, max_height)` is pure: it returns the exact
static requirement in wasm pages (data + shadow stack + workspace arena) or a
negative error code. Memory is imported, so JS must fix the real memory's
maximum before instantiation: it instantiates a throwaway instance over a
minimal scratch memory, queries `vip9r_required_pages`, and sizes the real
memory as required pages plus a flat 8 MiB packet-tail budget
(`sessionMaxPages` in wasm-env.ts; anchors: largest 1080p corpus packet
504 KiB, level 4.1 CPB 3 MiB, lossless keyframe ~3.1 MiB — a multi-frame
lossless superframe is the accepted fail-loud RESOURCE_LIMIT case).

The result block is a `u32` table in wasm memory. `reserve_input` publishes the
packet-tail pointer and capacity; JS copies one complete demuxed VP9
packet/superframe there; `begin_packet(len)` splits it into up to 8 coded-frame
ranges; `decode_next` consumes exactly one range and records `has_output`,
`packet_done`, dimensions, and native Y/U/V plane descriptors.

Wasm lays out the fixed workspace once from the instance max dimensions:
currently one current I420 frame slot plus 8 reference slots, followed by two
packed mode-history slots, then the growable packet tail. The packet tail is the
only region `reserve_input` may grow. Persistent wasm state stores offsets,
ranges, and layout descriptors, not long-lived Rust slices; slices are formed
only inside exports after bounds checks. JS must refresh `memory.buffer` views
after `reserve_input`, and output plane descriptors are ephemeral until the next
wasm decoder call.

## Measurement

Performance work has two gates:

- does this revision decode correctly?
- is this revision faster on the target path?

Correctness bring-up runs through wasm/d8 so unit tests, ABI checks, and golden
checks exercise the target build. Host d8 is the cheap parity and smoke loop.
For performance results, run device d8 A/B interleaved against the current
baseline with enough repetition to report a credible delta. Targeted
microbenchmarks are allowed when a full-decode result points at a hotspot; their
wins only count after reconfirming full-decode wall time.

Only log timings from harness-built wasm; ad-hoc builds are for exploration, not
the record.

### Devices and timing protocol

Probed 2026-07 on the two connected targets:

- **Pixel 9a (tegu, arm64):** 4×Cortex-A520 @1.95 GHz (cpu0-3), 3×A720
  @2.6 GHz (cpu4-6), 1×X4 @3.105 GHz (cpu7). Rooted; su lives at
  `/debug_ramdisk/su`. Policy: root reads for info-gathering are fine, root
  mutation of device settings (governors, freq locks) is to be avoided.
- **Google TV Streamer (kirkwood, armeabi-v7a only):** 4×Cortex-A55 @2.0 GHz,
  one shared cpufreq policy. Not rootable. Supported dev target despite the
  weaker confidence protocol below. On identical content the A55 runs ~1.75×
  slower than the Pixel A520 (weaker core plus V8 arm32 codegen).

Empirical behavior that shapes the protocol:

- Stock governors ramp the pinned core to max within a sample or two and hold
  it there under single-core d8 load on both devices. Run-to-run spread is
  ≤1% with no CPU control at all; back-to-back baseline/candidate runs of
  identical wasm agree within 0.02–0.5%. A/B deltas ≥~2% are credible from a
  single daemon bench run.
- A bench submission runs four counterbalanced runs — B1, C2, C3, B4
  (baseline, candidate, candidate, baseline) — so both sides average the same
  position and monotone thermal/position drift cancels. The headline number
  is `bench.corrected_delta` = mean(C2,C3)/mean(B1,B4) − 1 over
  `measurement.msPerFrame` (negative = candidate faster).
  `bench.baseline_spread` = B4/B1 − 1 is a built-in no-op control: a delta
  comparable to the spread is noise, not a result. Motivation: with the old
  baseline-then-candidate order the second position read ~2% slow cool-start
  and ~13% slow heat-soaked on the X4 (a no-op candidate reproduced it, so
  it was position bias, not a code delta). `--no-op-control` remains the
  explicit way to measure pure harness noise.
- The Pixel X4 heat-soaks: ~6% ms/frame degradation over 8 minutes of
  continuous decode at 89–94 °C on the BIG sensor, while `scaling_cur_freq`
  reports a constant 3105 MHz and `scaling_max_freq` never clamps. cpufreq
  telemetry is therefore a false-negative throttle signal on the Pixel. The
  honest signals are temperature and the timing drift itself (per-pass
  timings). Drift is ~0.1–0.2% per 10 s, so adjacent A/B runs cancel it;
  absolute margin numbers carry a ±6% thermal-state error unless started cool.
  BIG recovers 94→45 °C within ~30 s of idle.
- Temperature without root: the `dumpsys thermalservice` section
  `Current temperatures from HAL` tracks the root-only sysfs sensor
  (`/sys/class/thermal/thermal_zone0`, type `BIG`) within a few °C. The
  `Cached temperatures` section and derived thermal status are event-driven
  and can be stale by tens of °C; never use them. `dumpsys thermalservice`
  works on the streamer too (its sysfs thermal zones are what shell can't
  read); its HAL section has no `BIG`, but reports a CPU-type `soc_max`
  sensor. The streamer showed flat timings under 3.5 min of sustained load.
  Sustained 4-core load (2026-07-04, 10 min of pinned decode on all four
  cores): no frequency derating — policy0 `scaling_cur_freq` held 2.0 GHz
  throughout; `soc_max` rose 28 → 47 °C and was still flattening, thermal
  status 0. Cross-core contention costs ~5% on a measured core (jellyfish
  bench on cpu:0 with independent decode loops on cpus 1-3: 193.8 vs 184.6
  ms/frame solo) and roughly 1% of extra spread, so 4-core scaling on this
  device is compute-bound, not bandwidth- or thermally-bound.
- Confidence instrumentation (in every daemon device response and bench
  report): the daemon brackets each device d8 run with pinned-CPU
  `scaling_cur_freq` and the HAL temperature (`BIG`, else first CPU-type
  sensor), attached per run as `telemetry.{freq,temp}_{start,end}_*`;
  `temp_start_c` drives the cool-start rule for absolute margins. Bench
  responses carry `minPassMs`/`maxPassMs` plus measurement
  `passMsQuarterMeans` (mean pass wall time per quarter of the run, in
  order) — pass drift is the throttle/contention signal that works on both
  devices: monotone soak reads as a rising staircase, a throttle event as a
  step. Warmup pass arrays are JIT ramp and are dropped from responses; full
  raw `passMs` arrays persist in the device run_dir result files. Freq
  brackets sample outside the run and usually
  show idle governor state; they catch pin/policy mistakes, not throttling.
- The bench runner's per-pass deadline equals `targetMs` (5 s), and the
  binding pass is the md5 validation pass (md5 overhead ≈ +15% on X4, +60% on
  A55 vs a measurement pass). At 720p this caps decode windows at roughly
  ≤16 output frames on the X4, ≤4 on the A520, ≤2 on the A55 at current
  decoder speed. Decision: keep the 5 s limit — optimization is expected to
  bring realistic windows into range, and bloating run times for extra
  precision is not wanted. Revisit only if it blocks real work.

Margins are measured against the 33.3 ms/frame 720p30 budget. Starting-line
(2026-07, small windows, warm device): X4 ~147 ms/frame, A720 ~207, A520 ~845,
A55 ~1489.

The daemon binds one fixed socket (`temp/perf/vip9r-perf.sock`) and serves a
host queue plus one queue per `--serial` device, each with its own worker:
runs on different devices proceed in parallel while access to any one device
stays serialized. `serve` sorts device serials, so indices are stable for a
given set of connected devices. Submissions address a device by index (`--device N`, or
`VIP9R_PERF_DEVICE` inside grinder sandboxes) and state the CPU pin per
request; timed kinds (bench, microbench, profile) require an explicit pin,
validate/tests default to `any`. Pins take sets — `cpu:N`, `cpu:0-3`,
`cpu:0,2,4-5`, or `mask:HEX` — verified against the taskset's actual
`Cpus_allowed_list`; freq telemetry reads the first pinned CPU (both target
clusters share one cpufreq policy). Validate, bench, and profile requests
take a `pool` boolean (`--pool`/`--no-pool` on `vip9r-perf-submit`, defaulted
from `VIP9R_PERF_POOL` set by `grinder run --pool`; `--pool` on
`vip9r-corpus-golden.py` and on the wasm-golden runner itself), which
spawns the 3-worker pool and calls `vip9r_pool_activate` per decoder
instance; bench mode terminates each pass's pool before the next pass's
decoder so retired shared-memory reservations are released. Pool is never
inferred from the pin set and is rejected on other request kinds; responses
echo `pool` at the run summary and runner-report levels. Bench submissions carry their own baseline
wasm — `--baseline FILE`, else `VIP9R_PERF_BASELINE` (always set inside grinder
sandboxes; `grinder run` requires `--baseline`) — or say `--no-op-control` to A/B the
candidate against itself for a harness-noise reading. There are no quiet
fallbacks: a device request without an index or a bench without a stated
baseline is an error. The daemon holds no baseline state, so merges do not
force a restart; wasm pushes are cached per device by blob hash for the
daemon's lifetime. Grinder sandboxes bind the
socket's parent directory, so a daemon restart while a grinder runs only
produces connection-refused errors during the gap instead of permanently
severing the sandbox.

### V8 artifacts

The devshell pins Google-published V8 canary bundles. Operational details for
bumping those pins, running Android `d8` under qemu user emulation, and
extracting ARM Wasm assembly live in [`docs/d8.md`](d8.md).

## Threads (M6 scoping, 2026-07-03)

Content evidence (keyframe-header probe over the corpus): every 720p realworld
clip is coded with 4 tile columns (`tile_cols_log2=2`, the max at 1280w) and
`frame_parallel_decoding_mode=1` (no backward probability adaptation). YouTube
720p tracks are 4 columns with `frame_parallel=0`, 480p tracks are 2 columns;
`bear-vp9.ivf` is single-tile. Four tile columns match the A55's four cores,
and tile columns parallelize entropy decode — the cost share nothing else
touches.

The tile loop is already parallel-shaped: `parse_tile` (tile_syntax/mod.rs)
builds a self-contained `TileParser` per tile — own `BoolDecoder`,
stack-resident scratch — and left availability, MV candidate search, and intra
above-right gathering are all tile-column-scoped, so there are no cross-tile
reads of in-progress state. Shared mutable state across the tile loop is
exactly four things: `SyntaxCounts` (per-thread + merge; dead work when
adaptation is off, as on the whole `frame_parallel=1` perf corpus),
`TileModeContexts` (tiles touch disjoint column ranges of frame-width arrays),
the current-frame planes, and the mode grid. The last two are disjoint column
bands over row-interleaved storage — safe splits can't express that, so
parallel tiles need contained-unsafe disjoint views. `loop_filter_frame` runs
frame-wide after the tiles and crosses tile edges by spec; it parallelizes
separately as an SB-row wavefront.

Platform mechanics (verified by the 2026-07-03 ABI spike; the build is
threaded unconditionally, single ABI, old-ABI baselines retired):

- `+atomics,+bulk-memory` requires `core` built with the same features —
  prebuilt core objects make lld reject `--shared-memory`. Wiring:
  `rust/.cargo/config.toml` carries `[unstable] build-std = ["core"]` plus the
  target features and link args; stable cargo honors `[unstable]` only when
  `RUSTC_BOOTSTRAP=1` is in cargo's own environment (config `[env]` does not
  reach cargo itself), so the devshell, the wasm-tool wrappers, the harness
  scripts, and the grinder podman env all export it. A missing export fails
  loudly at link time. `compiler-builtins-mem` is NOT needed: with
  +bulk-memory LLVM lowers memcpy/memset intrinsics to memory.copy/fill and
  the linked module has no memcpy import (rustc 1.96.0). build-std adds ~2-3s
  of core compile to a cold build; grinder sandboxes always cold-build.
- Link `--shared-memory --import-memory --export-memory --max-memory=4GiB`.
  Re-exporting the imported memory keeps `exports.memory` and the whole wasm
  ABI intact; only instantiation sites changed. The 4GiB declared max is a
  ceiling only — each JS frontend creates the shared `WebAssembly.Memory`
  (`createVip9rMemory` in wasm-env.ts) and provides a workload-sized maximum
  (exact static requirement from `vip9r_required_pages` + 8 MiB packet tail;
  see the wasm boundary section), because V8 reserves the provided maximum
  upfront for shared memories — address space that must actually fit on
  arm32. TextDecoder rejects SAB-backed views; wasm-env copies before
  decoding logs.
- Measured cost of the shared-memory build, single-threaded (counterbalanced
  no-op benches vs logged numbers): X4/arm64 none (jelly 9.4 ms/f vs ~10.4
  logged); A55/arm32 +5% jelly / +8.5% BBB. Mechanism (profile + asm diff):
  V8 disables its memory-size instance cache for shared memories, so every
  bounds-check region reloads the size — decode_block `ldr [instance,#size]`
  65 → 556, +19% trap branches, +8.8% bytes; decode_residual 219 → 999 —
  a serialized load→sub→cmp→bcs chain per guarded access that the in-order
  core can't hide (X4 hides it entirely). Regression is concentrated in
  scalar entropy code (decode_block +32% absolute, mode-info +18%); simd
  kernels and libc copy cost are flat (relaxed-memcpy theory falsified;
  memory *base* is now a hoisted constant, 82 reloads → 1). No atomics
  emitted. Not source-fixable; size is monotonic so a future V8 could
  legally re-cache it. initial == maximum does not help (measured null —
  no constant-folding of shared size). A55 first-pass TurboFan compile is
  ~3.7s, so full-window bench warmup validation trips its 5s deadline —
  A55 benches use `--frames` windows (campaign protocol anyway).
- Shadow-stack binding (landed 2026-07-03). `__stack_pointer` is a
  per-instance mutable i32 wasm *global* (out-of-band, not a memory word)
  whose init value is baked at link time, so every instance over one shared
  memory starts aliasing the coordinator's stack-first 0..1 MiB region. The
  link exports the global (`--export=__stack_pointer`); JS rebinds a worker
  instance with `instance.exports.__stack_pointer.value = top` immediately
  after instantiation — no asm shim, and no wasm executes before the write:
  the only pre-rebind code is lld's `__wasm_init_memory` start function,
  stack-free by ABI construction (verified by disassembly). No TLS in the
  module (`__tls_base` init-0, no `__wasm_init_tls`), so the stack pointer is
  the only per-instance binding.
- Layout: fixed addresses so the worker stack tops are ABI constants and no
  code runs to learn them. Coordinator stack 0..1 MiB (linker stack-first;
  overflow wraps below zero and traps; size pinned by `-zstack-size=1048576`,
  the rustc default), three 1 MiB worker regions fixed at 1..4 MiB (tops
  0x200000 / 0x300000 / 0x400000; 4 threads hardcoded), data pushed to 4 MiB
  by `--global-base=4194304`. `arena_bounds` asserts `__heap_base` cleared
  4 MiB so a dropped flag fails loudly at init; `vip9r_required_pages` covers
  the stacks since they sit below the data + arena it already reports, and
  `INITIAL_PAGES` in wasm-env.ts covers the grown declared minimum
  (~4.05 MiB). Worker stacks have no overflow trap: overflow walks down into
  the neighboring stack; accepted, 1 MiB is the stack the whole decoder
  already fits in. Rejected alternatives: importing the global (value fixed
  before instantiation) is PIC-ABI-only — recompile-the-world plus per-access
  `__memory_base` indirection on a table-heavy decoder; a pure
  `vip9r_worker_stack_top()` export either executes compiler-generated code
  on a not-yet-rebound instance or relays the value through the coordinator's
  spawn message; lld cannot synthesize custom exported const globals.
- `__wasm_init_memory` once-guard mechanics (verified by disassembly): atomic
  cmpxchg on a flag word linker-placed below `__data_end`; the winner
  memory.inits the passive `.rodata`/`.data` segments, stores 2, notifies; a
  racer that observes 1 blocks in `memory.atomic.wait32` (traps on the
  browser main thread — instantiate the coordinator to completion before
  spawning workers, after which they hit the flag=2 skip path); latecomers
  skip straight to `data.drop`.
- Worker pool mechanics (settled 2026-07-03, pre-implementation):
  - Only the coordinator instance may touch the static `SESSION`. Blocking
    waits are illegal on the browser main thread — the coordinator lives in
    a dedicated worker (the demo already decodes in one; d8 `Worker`s allow
    blocking).
  - Control block is an ordinary Rust `static` — statics land in `.data`
    above the worker stacks and below `__heap_base`, so workers reference it
    symbolically at its link-time address and `WorkspaceLayout` is untouched
    (it exists only for variable-size state). It is the only shared-mutable
    static. Contents: `epoch` AtomicU32 go-word, `remaining` AtomicU32 join
    counter, three per-worker job slots (tile descriptor/index;
    `TileParserConfig` is identical across tiles so one shared field), and
    per-worker scratch *pointers* carved by the coordinator from its
    `WorkspaceLayout` — workers consume regions, never layout math (and the
    arena keeps paying for scratch via `vip9r_required_pages`).
  - Protocol: coordinator fills job slots, sets `remaining` to
    WORKER_COUNT, bumps `epoch` with Release, notifies; workers
    Acquire-load `epoch` and `memory.atomic.wait32` on the seen value (the
    compare-and-block closes the lost-wakeup race; spurious wakes re-loop).
    Every worker acknowledges every wave — `fetch_sub(1, Release)` on
    `remaining` after it is done reading its slot, job or not — and the one
    reaching zero notifies; the coordinator Acquire-waits after decoding
    its own share. Counting acknowledgements rather than jobs is
    load-bearing: a join that returned while an idle worker was still on
    its way to read a None slot would race the next dispatch's slot
    rewrite (torn `Option<Job>` read). All cross-thread pixel / mode-grid
    handoff rides those two Release/Acquire edges — no per-field atomics.
  - Teardown is JS `Worker.terminate()`: safe while the worker is parked in
    `memory.atomic.wait32` and at process exit (d8-probed 2026-07-03); no
    wasm-side shutdown path exists.
  - Activation (2026-07-03): the pool is all-or-nothing, 0 or 3 workers —
    join counts acknowledgements from every worker, so a partial pool hangs,
    and the fixed stack layout plus the fewer-cores degrade decision
    (oversubscribe, correctness over performance) leave no client for which
    1-2 workers is the right answer. A frontend that spawned all three
    workers calls `vip9r_pool_activate` once; the flag is a pool.rs static,
    default off, and `dispatch` asserts it. Frontends that spawn no workers
    (wasm unit tests outside `::pool::`, harnesses without a pool flag) stay
    on the serial path. Activation does not wait for worker startup: a
    worker whose first `epoch` load happens after a dispatch sees the bumped
    value and reads its slot.
  - Frontend policy: web always spawns (nested workers from the decode
    worker; Cobalt is Chromium-based and trusted for nested workers). d8 /
    perf harness use an explicit pool flag in request config — never
    inferred from the CPU pin set, matching the no-quiet-fallbacks rule and
    keeping serial-on-4-cores and oversubscribed-on-3-cores (Pixel A720
    cluster) runs expressible.
  - Dispatch: no job queue. Coordinator dispatches an initial wave of one
    tile per worker, decodes one tile itself, serially mops up any
    remainder, then joins. Modal cases degrade cleanly: 4 tiles = wave of 3
    + own tile + empty remainder; 2 tiles = wave of 1 with two None slots;
    single-tile clips skip dispatch/join entirely. >4-tile clips serialize
    the excess on the coordinator — accepted until content demands a queue.
- Spawn frontends (landed 2026-07-03): the stack-layout ABI constants live
  in `js/src/wasm-driver/stack-layout.ts`, shared by both spawners. d8:
  `wasm-driver/pool.ts` (string-source worker, constants interpolated); the
  wasm-tests runner spawns + activates for `::pool::` tests. Web:
  `web/pool.ts` + `web/pool-worker.ts` (module worker, sync instantiation so
  layout-assert throws reach the spawner's `onerror`), spawned from the
  demo's decode worker on every playback, then `vip9r_pool_activate` — the
  nested-worker + rebind path is exercised even while decode is serial.
  Verified on Chrome 2026-07-03: three parked pool-worker targets during
  playback, clean console.
- `VideoFrame` construction from SAB-backed views verified on Chrome and
  Cobalt (user-tested, 2026-07-03).

Expectations: ffvp9 frame threading measured ~2.9x on the A55 quad (one
cpufreq policy, in-order cores) — treat that as the realistic scaling anchor.
At ~2.7-3x, BBB (109 ms/frame at campaign wrap) lands under its 40 ms budget;
jellyfish (144 ms/frame) lands ~1.5x over 33.3 ms. The single-thread entropy
levers are exhausted (2026-07-03: parse→dequant fusion merged, branchless
bool decision measured null).

Frame-parallel decode is out of scope (decided 2026-07-03, complexity). It
would break the one-frame-owns-all-mutable-state invariant: per-frame
snapshots of probability contexts / segmentation map / mode-MV grid, per-row
reconstruction-progress gating on reference reads, two live current-frame
workspaces, double-buffered packet input, +1 frame latency in the ABI. It is
also inert on flag=0 content: with backward adaptation on, frame N+1's
entropy state needs frame N's counts, which the fused parse produces only at
frame completion — and every probed YouTube track (all four IDs, 480p and
720p) codes `frame_parallel_decoding_mode=0` with `refresh_frame_context=1`
(per-file header probe, 2026-07-03). A jellyfish-class gap that survives tile
+ loop-filter parallelism is accepted rather than chased.

To keep flag=0 content honest in threads-era measurement, the perf bench set
gains `youtube/mN9_buCmKLE` f247 720p30 (1280x720, ~1.5 Mbps, 36k frames,
corpus-golden green) as the flag=0 lane. Counts accumulation and probability
adaptation are live on this clip — unlike the `frame_parallel=1` realworld
clips, where adaptation never runs — so "skip counts when adaptation is off"
and per-thread counts merge both stay measured instead of optimized against
a corpus that cannot see them.

## Work shape

- The interactive session owns intent, harness setup, measurement, and review.
- Implementation agents should receive narrow, reviewable tasks with the needed
  spec excerpts, tests, and oracle handle.
- Rust implementation state lives in the self-contained Cargo workspace under
  `rust/`. Keep maintainer/orchestrator tooling outside that tree unless the
  implementor needs it for the task.
- Grinder sandboxes make `rust/` the repo root, while the main checkout's VCS
  root is the project root. Import mechanics live in `docs/orchestrator.md`.
- Keep task boundaries provisional. Split by whatever makes correctness,
  measurement, and review easiest at the time.
- Prefer grinder tasks smaller than the first packet front-end handoff. That
  bootstrapped useful structure but landed about 1k LOC; ordinary implementor
  tasks should be easier to review in isolation.
- Good grinder packets tend to change one kind of thing: one parser primitive,
  one syntax-table slice, one data model needed by the next slice, or one
  measured optimization. Borderline packets mix modeling choices, semantic
  changes, and fixture churn. Split before a task asks the implementor to both
  invent a shape and consume it broadly.
- Size by review risk, not lines. A few hundred lines of mechanical tables or
  local tests may be fine; a small diff can still be too large if it commits the
  decoder to a hard-to-unwind interpretation of the spec.
- Parallel grinders are for disjoint write sets or independent investigations.
  Sequential is the default when tasks share parser state, probability tables,
  frame storage, or other files where merge conflicts would hide review issues.
- Do not turn temporary boundaries into architecture unless they survive contact
  with implementation and measurement.

## Record

- `docs/tracker.md` is the ordered backlog and current frontier.
- `docs/log.md` is the human-facing implementation/optimization record. Entries
  should cite either a correctness milestone or a measured performance delta.
- Add extra notes only when they prevent rediscovering a real decision or
  failure mode. Avoid ledgers for speculative module pieces.

## Corpus

Test media lives at `/bulk/vip9r` on the host and inside the grinder sandbox.
Each vector has a `.md5` golden.

- **Correctness:** `libvpx/` conformance vectors (profile 0 / 8-bit subset, IVF
  and WebM) plus `chromium/bear-vp9.ivf` as the default IVF smoke target.
- **Performance:** `realworld/` 720p clips with distinct character (high-motion,
  film grain, screen content, talking head) — all `frame_parallel=1` — plus
  `youtube/mN9_buCmKLE` f247 720p30 as the flag=0 lane (`frame_parallel=0`,
  `refresh_frame_context=1`: probability adaptation runs on this clip and on
  no other perf clip).

## Open design questions

- Which intermediate checks, if any, are worth adding after the first failures.
- Device-time and token budget per optimization pass.

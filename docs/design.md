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
- minih264 is used for the showcase path. It is not the current design center.
- SIMD, relaxed SIMD, workers, allocator shape, and internal boundaries are
  choices to earn with evidence, not decisions to pre-bake into the docs.
  - 2026-07: wasm simd128 is in scope for M3. It is baseline in shipped Chrome
    and V8 lowers it to NEON on both device targets (arm64 and the streamer's
    arm32), so it is portable, not CPU-specific tuning. Relaxed SIMD and
    workers remain M6 stretch.

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
binding: `vip9r_result_ptr`, `vip9r_init`, `vip9r_reserve_input`,
`vip9r_begin_packet`, and `vip9r_decode_next`. Mutating exports return `0` for
success or a negative error code; those codes are binding details, not a stable
external API.

The result block is a `u32` table in wasm memory. `reserve_input` publishes the
packet-tail pointer and capacity; JS copies one complete demuxed VP9
packet/superframe there; `begin_packet(len)` splits it into up to 8 coded-frame
ranges; `decode_next` consumes exactly one range and records `has_output`,
`packet_done`, dimensions, and native Y/U/V plane descriptors.

Wasm lays out the fixed workspace once from the instance max dimensions,
currently one current I420 frame slot plus 8 reference slots, followed by the
growable packet tail. The packet tail is the only region `reserve_input` may
grow. Persistent wasm state stores offsets, ranges, and layout descriptors, not
long-lived Rust slices; slices are formed only inside exports after bounds
checks. JS must refresh `memory.buffer` views after `reserve_input`, and output
plane descriptors are ephemeral until the next wasm decoder call.

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
- Confidence instrumentation (in every daemon device response and bench
  report): the daemon brackets each device d8 run with pinned-CPU
  `scaling_cur_freq` and the HAL temperature (`BIG`, else first CPU-type
  sensor), attached per run as `telemetry.{freq,temp}_{start,end}_*`;
  `temp_start_c` drives the cool-start rule for absolute margins. Bench
  reports carry per-pass wall times (`passMs`, `minPassMs`, `maxPassMs`) in
  warmup and measurement — pass drift is the throttle/contention signal that
  works on both devices. Freq brackets sample outside the run and usually
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

The daemon binds one fixed socket (`temp/vip9r-perf.sock`) and one device per
`serve`; switching devices is a daemon restart.

### V8 artifacts

The devshell pins Google-published V8 canary bundles. Operational details for
bumping those pins, running Android `d8` under qemu user emulation, and
extracting ARM Wasm assembly live in [`docs/d8.md`](d8.md).

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
  film grain, screen content, talking head). Keep at least one held out for
  review.

## Open design questions

- Which intermediate checks, if any, are worth adding after the first failures.
- Device-time and token budget per optimization pass.

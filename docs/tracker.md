# vip9r — tracker

The "what's next" surface. Co-maintained by the user and the agent. Keep this
as an ordered backlog, not an architecture commitment.

## How to read this

Every item carries a **next-step tag** — what kind of action unblocks it:

- `[define]` — needs more specification (user + agent edit AGENTS.md / design.md)
- `[procure]` — needs an external input (user supplies: corpus, device, specs)
- `[tool]` — needs trusted harness/oracle code (built at the top layer, never by
  grinders)
- `[grind]` — ready for a codex implementor / optimizer

Status: `todo` / `wip` / `done`. "What's next" = the lowest open milestone's
unblocked items; the tag says who acts.

**Two independent tracks until M2.** The correctness path runs on host d8
against conformance vectors and generated goldens; it needs no device. The
device path (rooting, pinning, the device d8 endpoint) only gates the optimize
campaign. Don't sink time into device plumbing before the decode-correct loop
exists.

## Now

M0 bootstrap. Nothing built yet. The first unblocks are `[procure]`
(conformance vectors) and `[tool]` (wasm skeleton + host d8 build + frame-output
golden harness) on the correctness path. Device path can wait.

---

## M0 — Bootstrap (top layer, trusted)

### Correctness path (host-only; gates M1)
- `[tool]` todo — freestanding `no_std` wasm skeleton with the minimal decode API
- `[tool]` todo — host d8 build + JS driver
- `[tool]` todo — golden harness: conformance vectors and libvpx-generated
  frame-output goldens
- `[procure]` todo — conformance vectors, profile 0 / 8-bit subset (user)
- `[define]` todo — first-failure debugging strategy: which intermediate checks
  are worth adding, if frame-output diffs are too coarse
- `[define]` todo — implementor task template: spec excerpts, fixtures, expected
  outputs, and oracle instructions
- `[tool]` todo — grinder container environment (nix, host store shared)
- `[define]` todo — grinder commit/workspace mechanics under jj

### Device path (gates M2; deferrable)
- `[procure]` todo — root the Pixel 9a (user)
- `[procure]` todo — performance corpus: ~4 distinct-character 720p clips, one
  held out (user)
- `[tool]` todo — oracle daemon: owns adb + device, queues access
- `[tool]` todo — simulator d8 build (`v8_target_cpu="arm64"`, `--print-wasm-code`)
- `[tool]` todo — device d8 endpoint: taskset big-core pin, cpufreq lock, A/B
  interleave with confidence intervals
- `[define]` todo — device-time + token budget per explore step, and cadence

## M1 — Decode, correct (implement campaign)

VP9 profile 0 / 8-bit from the spec, passing the conformance subset on full
frames. Scalar, no performance bar. Gated on the M0 correctness path.

- `[grind]` todo — implement the smallest useful decode slice and make it
  reviewable
- `[grind]` todo — add intermediate checks only where full-frame failures are not
  local enough
- `[grind]` todo — conformance subset green on full frames

## M2 — Decode, fast (optimize campaign)

Sustained 720p30 on the pinned big core. Gated on M1 + the M0 device path.

- `[grind]` todo — optimize measured full-decode hotspots; confirm wins against
  full-decode wall time

## M3 — Encode

minih264 in, MSE-playable H.264 out. Fitness gains a VMAF/size floor.

- `[grind]` todo — integrate minih264 behind the libc shims
- `[define]` todo — VMAF/size floor

## M4 — Chrome demo

Real browser on the phone, MSE player page, full VP9→H.264 loop.

- `[tool]` todo — demo page (vite/TS), WebM demux + ISO BMFF mux, MSE append

## M5 — Stretch

- relaxed-simd · threads · little-core (Cortex-A520) · 1080p

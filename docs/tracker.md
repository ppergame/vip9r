# vip9r — tracker

The "what's next" surface. Co-maintained by the user and the agent. Detail lives
in `docs/design.md`; this is the ordered backlog and current frontier.

## How to read this

Every item carries a **next-step tag** — what kind of action unblocks it:

- `[define]` — needs more specification (user + agent edit AGENTS.md / design.md)
- `[procure]` — needs an external input (user supplies: corpus, device, specs)
- `[tool]` — needs trusted harness/oracle code (built at the top layer, never by
  grinders)
- `[grind]` — ready for a codex implementor / optimizer

Status: `todo` / `wip` / `done`. "What's next" = the lowest open milestone's
unblocked items; the tag says who acts.

**Two independent tracks until M2.** The correctness path (decode-correct) runs
entirely on host d8 against conformance vectors + libvpx goldens — it needs no
device. The device path (rooting, pinning, the device d8 endpoint) only gates the
optimize campaign (M2). Don't sink time into device plumbing before kernels are
landing bit-exact.

## Now

M0 bootstrap. Nothing built yet. The first unblocks are `[procure]` (conformance
vectors) and `[tool]` (wasm skeleton + host d8 build + golden harness) on the
correctness path. Device path can wait.

---

## M0 — Bootstrap (top layer, trusted)

### Correctness path (host-only; gates M1)
- `[tool]` todo — freestanding `no_std` wasm skeleton + DSP function-table seam
- `[tool]` todo — host d8 build + JS driver
- `[tool]` todo — libvpx golden harness: per-kernel goldens at the DSP-kernel
  boundary (libvpx specifics TBD — see boundary analysis), full goldens at frame
  output
- `[tool]` todo — capture mode: shim the DSP table, snapshot stage-input blobs
- `[procure]` todo — conformance vectors, profile 0 / 8-bit subset (user)
- `[define]` todo — golden harness boundary analysis: do the leaf kernels tap at
  libvpx's `vpx_dsp` rtcd surface, and where does that surface not line up — e.g.
  `*_add` fuses transform+residual-add, dequant/detokenize sit above it, inter
  recon spans `vp9/common`; parse-half verification strategy (full-frame +
  tappable structs)
- `[define]` todo — grinder spec template for implement mode (spec excerpts +
  per-kernel notes)
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

VP9 profile 0 / 8-bit from the spec, kernel by kernel, each bit-exact against its
golden; conformance subset passing on full frames. Scalar, no performance bar.
Gated on the M0 correctness path. Per-kernel ledger materializes here.

- `[grind]` todo — reconstruct half (leaf kernels), verified at stage granularity
- `[grind]` todo — parse half (bool decoder, header, modes, MVs, context
  adaptation), verified at full-frame granularity
- `[grind]` todo — conformance subset green on full frames

## M2 — Decode, fast (optimize campaign)

Sustained 720p30 on the pinned big core. Gated on M1 + the M0 device path.

- `[grind]` todo — rice + SIMD128 the kernels, per-kernel, against measured deltas

## M3 — Encode

minih264 in, MSE-playable H.264 out. Fitness gains a VMAF/size floor.

- `[grind]` todo — integrate minih264 behind the libc shims
- `[define]` todo — VMAF/size floor

## M4 — Chrome demo

Real browser on the phone, MSE player page, full VP9→H.264 loop.

- `[tool]` todo — demo page (vite/TS), WebM demux + ISO BMFF mux, MSE append

## M5 — Stretch

- relaxed-simd · threads · little-core (Cortex-A520) · 1080p

# vip9r — tracker

The "what's next" surface. Co-maintained by the user and the agent. Keep this as
an ordered backlog, not an architecture commitment.

## How to edit this

- Prefix a task with `user:` only when it needs external input or a decision
  from the user.
- Move bullets to change priority. `Now` is the default work queue.

## Now

M0 bootstrap, host-only correctness path. Nothing built yet.

- [x] user: VP9 profile 0 / 8-bit spec source
- [x] user: conformance vectors for the profile 0 / 8-bit subset
- [x] prebuilt d8 binaries for host and ARM
- [ ] freestanding `no_std` wasm skeleton with a minimal decode API
- [ ] golden comparison loop for conformance vectors and frame-output goldens
- [ ] implementor handoff packet for the first decode slice
- [ ] implementation-agent sandbox and workspace plan for the first handoff

## Waiting

Deferrable external inputs for M2 performance work.

- [ ] user: root the Pixel 9a
- [ ] user: performance corpus representative of the 720p target, with a held
      out clip

## Later

### M0 device path — gates M2

- [ ] device oracle that serializes access to adb and hardware
- [ ] host and ARM d8 paths for wasm inspection and timing, JIT tier control
- [ ] repeatable device timing protocol with CPU control and confidence checks

### M1 — Decode, correct (implement campaign)

VP9 profile 0 / 8-bit from the spec, passing the conformance subset on full
frames. Scalar, no performance bar. Gated on the M0 correctness path.

- [ ] define and implement the first reviewable decode slice
- [ ] add intermediate checks only where full-frame failures are not local
      enough
- [ ] conformance subset green on full frames

### M2 — Decode, fast (optimize campaign)

Sustained 720p30 on the pinned big core. Gated on M1 + the M0 device path.

- [ ] optimize measured full-decode hotspots; confirm wins against full-decode
      wall time

### M3 — Encode

minih264 in, MSE-playable H.264 out. Fitness gains a VMAF/size floor.

- [ ] integrate minih264 behind the required wasm/libc boundary
- [ ] establish the VMAF/size floor

### M4 — Chrome demo

Real browser on the phone, MSE player page, full VP9→H.264 loop.

- [ ] demo page: vite/TS, WebM demux + ISO BMFF mux, MSE append

## M5 — Stretch

relaxed-simd · threads · little-core (Cortex-A520) · 1080p

# vip9r — design

Working notes on constraints and mechanics. AGENTS.md holds the project framing
and durable decisions; this file should stay small enough to guide the next
session without pretending the decoder architecture is settled.

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

## Correctness

The first correctness bar is simple: VP9 profile 0 / 8-bit conformance vectors
and selected corpus clips must decode bit-exactly at frame output.

libvpx-generated outputs are useful when the spec or test vectors are not
enough. Intermediate checks may be added when they make failures easier to
localize, but the tap points and granularity should follow the implementation we
actually have. Do not commit the project to libvpx component boundaries just
because they are available to instrument.

## Measurement

The trusted harness answers two questions:

- does this revision decode correctly?
- is this revision faster on the target path?

Host d8 is the cheap correctness and smoke-performance loop. Device d8 is the
ground truth for performance and should run A/B interleaved against the current
baseline with enough repetition to report a credible delta. Targeted
microbenchmarks are allowed when a full-decode result points at a hotspot; their
wins only count after reconfirming full-decode wall time.

Only rev-built runs — built by the trusted harness from a VCS revision — are
citable in `docs/log.md`. Ad-hoc blobs are for exploration.

### V8 artifacts

The devshell pins Google-published V8 canary bundles. Operational details for
bumping those pins, running Android `d8` under qemu user emulation, and
extracting ARM Wasm assembly live in [`docs/d8.md`](d8.md).

## Work shape

- The interactive session owns intent, trusted harness work, and review.
- Implementation agents should receive narrow, reviewable tasks with the needed
  spec excerpts, tests, and oracle handle.
- Keep task boundaries provisional. Split by whatever makes correctness,
  measurement, and review easiest at the time.
- Do not turn temporary boundaries into architecture unless they survive contact
  with implementation and measurement.

## Record

- `docs/tracker.md` is the ordered backlog and current frontier.
- `docs/log.md` is the human-facing implementation/optimization record. Entries
  should cite a revision and either a first-correctness milestone or a measured
  performance delta.
- Add extra notes only when they prevent rediscovering a real decision or
  failure mode. Avoid ledgers for speculative module pieces.

## Corpus

- **Correctness:** VP9 profile 0 / 8-bit conformance vectors.
- **Performance:** a small set of 720p clips with distinct character
  (high-motion, film grain, screen content, talking head). Keep at least one
  held out for review.

## Open design questions

- Minimal wasm/JS API needed for the first decode-correct loop.
- Which intermediate checks, if any, are worth adding after the first failures.
- Device-time and token budget per optimization pass.
- Implementation-agent workspace mechanics under jj.

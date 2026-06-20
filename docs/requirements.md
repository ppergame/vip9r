# vip9r

vip9r is an LLM-optimized software VP9 decoder targeting WebAssembly. It is a
work in progress.

## Premise

Suppose a Media Source based web app needs to play VP9 content on a device that
does not support VP9 hardware decode. The app could:

- Demux and decode WebM VP9 segments
- Encode in H264 and mux into ISO BMFF
- Append segments

### Objectives

- adequate performance with 720p30 video on a weak ARM streaming stick
- demo page showcasing playback
- implementation log `docs/log.md` showcasing decoder implementation and
  optimization progress. Compact and legible for human consumption

## Tech choices

- Demo webpage
  - vite, TypeScript, vitest
- Mux/demux
  - ad-hoc in TypeScript
- Wasm module
  - Safe Rust with unsafe blocks for platform integration, performance, minih264
    integration
  - wasm32-unknown-unknown
  - Explore: bumper allocator, reset every GOP
  - Streaming I/O at frame granularity
  - Bindings: manual, ad-hoc
  - Wasm features available in the default build of Chrome
    - Explore: relaxed-simd, workers
  - VP9 decoder
    - Profile 0, 8-bit
    - "clean-ish" room from spec, test vectors, test content
    - libvpx is the correctness oracle
    - libvpx source code is a debugging oracle
      - frame-level result not correct? perform differential debugging block by
        functional block
      - no peeking otherwise
      - vip9r is a "derived work" of libvpx
  - H.264 encoder
    - minih264
- Performance baseline
  - ffvp9, direct run through ADB
- Performance oracle
  - Companion socket server driven by CLI commands. Queues device access
    requests.
  - Devices
    - local machine
    - a connected Pixel 9a
    - streaming stick TBD
  - Inputs
    - d8 binary (native, ARM)
    - JS driver
    - Wasm module under test
  - Shell wrappers
    - Generate x86 or ARM asm for this wasm module
    - Benchmark this many frames for this video on host or device
    - Benchmark function slot X in this wasm module

## Testing strategy

- Unit tests: disposable, as needed to pin decisions and bug fixes. Not
  interested in full coverage
- Integration tests
  - Partial computation within a frame: nice-to-have. Remove if the test blocks
    a desired architecture change
  - Frame-level correctness: mandatory
  - Clip-level correctness: mandatory
  - H.264 output quality: nice-to-have

## Agent roles and scope

### You, the agent reading this

You are a "user agent" responsible for ensuring honesty both up and down the
stack. Example cues:

- did the user express requirements clearly and definitively?
- did the user provide required inputs like specs, test media, test devices?
- did the user agree to non-trivial design decisions?
- is the clean-ish room aesthetic observed?
- are decoder changes and optimizations representative of "production" use?
- are implementation agents happy with their prompts, inputs and tools?
- are functions, modules, changes and other artifacts well factored and easy for
  the user to review?

You do _not_ grind out a task at all costs. That's the job of the implementor
agent.

The user maintains this file which contains important decisions. You may suggest
additions or other changes. The user and you co-maintain the milestone and task
tracker `docs/tracker.md`. You maintain `docs/design.md` and any other documents
you need. The user doesn't generally look at them unless there is a
misunderstanding or process breakdown. Do keep in mind a principle of generative
minimalism: the docs should have enough information for a new session to deduce
what it needs. They are not summaries or narratives that becomes stale the
moment the session is done.

- Role: Tooling/harness engineering
  - Scope: a pending change
  - This is the default for normal interactive development with the user.
- Role: Orchestrator
  - Scope: roughly a context window's worth of revisions making incremental
    progress on VP9 decoder implementation or optimization
  - Comes up with optimization strategies
  - Manages codex implementation agents
    - Delegates bulk coding work
    - Reviews and merges changes
    - Makes minor code fixups
    - Crafts implementor instructions and prompts

### Codex implementor

Scope: one decoder implementation functional block or optimization strategy.
Roughly a commit's worth.

Sandboxed non-interactive implementation agent with limited inputs:

- specs and test media
- handle to performance oracle and instructions for use
- Toolchain
- Wasm module source directory

The sandbox's purpose is context management, not security. The implementor has
access to the common Nix store and is free to download the necessary specs. It
can also ask the orchestrator for tools to be added to the ambient context on
the next run.

# vip9r

[Live demo](https://vip9r.xzrq.net/)

vip9r is a software VP9 decoder written from scratch in Rust and targeting
WebAssembly.

Objective: 720p30 VP9 decode running in "real time"<sup>[1]</sup> on a Google TV
Streamer 4K, an extremely underpowered 32-bit ARM box.

> <sub><sup>1</sup>Decode only; rendering to a \<canvas\> doubles the frame time
> on Cobalt because it doesn't use a hardware overlay.</sub>

## Why?

The concept isn't new: GitHub is littered with Wasm ports of ffmpeg and libvpx.
I wanted to see how far a frontier LLM can get working purely from the spec and
device feedback. I instructed the agents not to look at existing implementations
and they didn't.

The model weights do contain a not-so-blurry-anymore snapshot of open source
software. I think the Wasm optimization target is far enough off the beaten path
to make this an interesting exercise.

## Results

First 500 frames of
[Stunning Coral](https://www.youtube.com/watch?v=mN9_buCmKLE) at 720p30. Decode
only.

Multiples of realtime, higher is better.

|                                           | Google TV Streamer 4K | Pixel 9a       | PC  |
| ----------------------------------------- | --------------------- | -------------- | --- |
| vip9r, [d8 shell](https://v8.dev/docs/d8) | 1.11×                 | 5.8×           | 24× |
| ffvp9 single core                         | 1.5×                  | 13×            | 30× |
| ffvp9 4 cores                             | 4.3×                  | <sup>[1]</sup> | 60× |
| _in browser:_ <sup>[2]</sup>              |                       |                |     |
| vip9r                                     | 1.04×                 | 8.5×           | 24× |
| ogv.js (libvpx Wasm)                      | 0.59×                 | 4.1×           | 11× |
| WebCodecs (libvpx)                        | 4.5×                  | 30×            | 83× |

<details>
<summary>Raw measurements, ms/frame</summary>

|                                           | Google TV Streamer 4K | Pixel 9a       | PC   |
| ----------------------------------------- | --------------------- | -------------- | ---- |
| vip9r, [d8 shell](https://v8.dev/docs/d8) | 30.09                 | 5.70           | 1.41 |
| ffvp9 single core                         | 22.7                  | 2.56           | 1.11 |
| ffvp9 4 cores                             | 7.82                  | <sup>[1]</sup> | 0.56 |
| _in browser:_ <sup>[2]</sup>              |                       |                |      |
| vip9r                                     | 32.1                  | 3.9            | 1.4  |
| ogv.js (libvpx Wasm)                      | 56.7                  | 8.1            | 2.9  |
| WebCodecs (libvpx)                        | 7.4                   | 1.1            | 0.4  |

</details>

> <sub><sup>1</sup>Pixel 9a has a single fast core. Threaded runs are
> slower.</sub>

> <sub><sup>2</sup>Google TV Streamer used
> [Cobalt](https://github.com/youtube/cobalt) with flags to enable TurboFan and
> SAB transfer. Chrome for the other two.</sub>

## Method

Much of the workflow is standard early 2026 vibe coding. I started with GPT-5.5
and switched to Fable 5 when it was re-released.

- I maintain a [requirements.md](docs/requirements.md) to say what I want in the
  project. Agents do not modify the file.
- Planning sessions: I ask the agent to record milestones and tasks in
  [tracker.md](docs/tracker.md)
- Implementation sessions: I tell the agent to work on a task. The model is
  welcome to chitter to itself in any other docs. I don't look at them.
- I review key boundaries like the Wasm ABI and memory layout, mutexes and
  overall slop level and make the commit myself.
- Each session runs in yolo mode in a
  [dev container](https://www.github.com/xzrq-net/claudepod). I'm trusting the
  sandbox with my life.

### Closing the agent [loop](https://media.xzrq.net/vip9r/loops_loops_loops_opt.png)

The latest models can more or less work autonomously on tasks with a clear
feedback signal. This project is 20% shop jigs and 80% code with well defined
success criteria. My role is to declare intent, set up the context, connect the
agent tooling and let the friendly robots do the work. In practice, I couldn't
resist tweaking prompts and arguing about the implementation details. There were
only a handful of autonomous multi-hour sessions. Perhaps Fable would've done
something reasonable with a simple "make me a decoder" prompt and a new car's
worth of API costs. As it is, this only cost about $3,256.97 of subsidized
tokens.

These were the critical tools / feedback mechanisms:

- Chrome dev tools MCP. Inspect headless Chrome, desktop Chrome, device Cobalt.
- All of nixpkgs. The agent runs any software it needs and saves me from being a
  `sudo apt install` janitor.
- Nix flake. The code is mostly hermetic and reproducible, excluding media but
  including upstream components like d8, a custom ffmpeg build, Android
  simulator runtime, ARM XMLs.
- [Specs!](docs/specs) I burned quite a few tokens running one-time conversions
  from VP9 spec and ARM PDFs to faithful markdown files. I also threw in Wasm
  core spec and Rust Wasm. The agents ignored the ARM docs and worked from the
  VP9 spec.
- A [CLI](scripts/vip9r-perf-daemon.py) to push static assets to test devices,
  run validation and benchmarks interleaved against the checked in baseline,
  collect profile traces and native assembly. I also added a microbenchmark ABI
  but the agents didn't find a use for it.
- An [orchestrator role](docs/orchestrator.md) overseeing a sandboxed
  [implementation subagent](scripts/grinder-system-prompt.md) to run codex with
  GPT-5.5 with a limited context. No JS, no project docs, only pure optimization
  grinding against a benchmark oracle. I'm not sure how much this reduced
  decision fatigue for the model and helped it focus on the task. It was
  certainly cheaper: Fable 5 has tight subscription quotas but a $200/mo OpenAI
  plan is basically unlimited.
- Orchestrator keeps a historical record in `docs/log.md`.

## Random thoughts

- Fable 5 is _really_ good. It is another step up in capabilities comparable to
  the Opus 4.5 release. The model is extremely sensitive to user intent,
  produces reasonable code and demonstrates good taste. It's adequate at writing
  subagent prompts. GPT-5.5 is a reasonable task subagent, but it had too much
  compliance RL for any metacognitive tasks.
  - Nonetheless, the agents didn't get anywhere near ffmpeg's repertoire of
    clever optimization tricks.

Fun facts:

- 32-bit ARM is a hostile optimization target for Wasm
  - Explicit memory bounds checks cause a 15% hit. 64-bit targets just reserve
    4GB worth of address space.
  - One session ran into a genuine codegen bug that I reported in v8 tracker
    https://issues.chromium.org/issues/531793716
- On 32-bit ARM, bionic libc packs a thread ID into the lower 16 bits of a futex
  address. This does not work well in qemu-user on a workstation that has been
  running for a while and racked up large PIDs. Fable diagnosed it in 20 minutes
  by comparing strace output. It would've taken me half a day.

## Attribution & Disclaimer

- [ogv.js](https://github.com/bvibber/ogv.js) by Brooke Vibber (MIT)
  - [libvpx](https://github.com/webmproject/libvpx) (BSD-3-Clause)

Everything in this repo is AI-generated except for this README and the prompt
files. This project is performance art. No scientific or commercial value is
intended.

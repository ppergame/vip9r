# vip9r wasm boundary redesign

Target design for replacing `vip9r-wasm`'s placeholder boundary. Promote durable
implemented decisions into `docs/design.md`.

## Current baseline

- `vip9r-core` exposes stateful packet decode:
  `Decoder::decode_packet(packet, sink)`.
- Shown frames are pushed synchronously through `FrameSink`; frame borrows are
  valid only during the callback.
- That callback packet API is current implementation shape, not a wasm-boundary
  constraint.
- Public core output is compact I420 in libvpx-md5 order: visible Y, then U,
  then V. Internal storage may use stride or padding.
- `vip9r-wasm` still exports placeholder helpers plus
  `vip9r_decode_frame(input_ptr, input_len, output_ptr, output_len)`, which
  ignores its pointers and returns `Unimplemented`.
- JS wasm bindings are empty.

## First boundary scope

One `WebAssembly.Instance` is one decoder session. Concurrent decoders use
separate wasm instances, not handles inside one shared linear memory.

The wasm boundary accepts one complete contiguous demuxed VP9 packet/superframe,
splits it into coded-frame ranges, and advances one coded frame at a time. Each
step returns zero or one ephemeral shown-frame descriptor.

In scope:

- decoder session setup, packet staging, packet splitting, and
  one-coded-frame stepping;
- native plane descriptors for shown VP9 frames;
- resource limits derived from `max_width` and `max_height`;
- d8/browser-callable integer exports with manual JS bindings.

Out of scope:

- WebM/network accumulation and timestamp policy. JS owns both.
- Multiple decoder handles inside one wasm instance.
- JS-owned decode buffers.
- Output queues, frame pin/release APIs, and delayed VP9 frame retention for a
  later encoder stage.
- H.264/minih264 integration.
- Workers or shared wasm memory.

## Export shape

Target exports:

```text
vip9r_result_ptr() -> u32

vip9r_init(max_width: u32, max_height: u32) -> i32
vip9r_reserve_input(len: u32) -> i32
vip9r_begin_packet(len: u32) -> i32
vip9r_decode_next() -> i32
```

Mutating exports return `0` for success and a negative value for failure. The
paired JS binding maps failures to JS errors; numeric error codes are not stable
API outside that binding.

`vip9r_result_ptr` returns the linear-memory offset of one per-instance result
block. `reserve_input` writes the packet staging pointer and capacity there. JS
copies the complete packet to that range, then calls `begin_packet(len)`.
`begin_packet` uses the active staging range rather than caller-supplied packet
pointers.

## Result block

The result block is an integer-only view into wasm memory used by the paired JS
binding after each export. It contains:

- input staging pointer and capacity after `reserve_input`;
- active packet byte count, coded-frame count, and zero-based coded-frame index;
- `has_output` and `packet_done` booleans for `decode_next`;
- decoded and render dimensions for a shown frame;
- Y/U/V plane offsets, byte lengths, and strides for a shown frame.

The paired JS binding treats frame descriptor fields as valid only after a
successful `decode_next` with `has_output != 0`. `decode_next` sets
`packet_done` on the step that consumes the final coded frame in the active
packet.

JS reacquires `memory.buffer`, typed-array views, and the result-block view
after every successful `reserve_input`; `memory.grow` can detach old host views
even though numeric wasm offsets remain stable.

## Session semantics

Phases:

```text
Uninit --init--> Ready --begin_packet--> PacketActive --packet_done--> Ready
```

Rules:

- `init(max_width, max_height)` is valid only in `Uninit`.
  - On success it lays out the fixed workspace and enters `Ready`.
  - On invalid config or resource failure it stays `Uninit`.
  - Changing limits requires a new `WebAssembly.Instance`.
- `reserve_input(len)` is valid only in `Ready`.
  - It grows only the packet tail if needed.
  - It does not create a Rust slice and does not change phase.
- `begin_packet(len)` is valid only in `Ready`.
  - It requires `len > 0` and `len <= input_capacity`.
  - It splits the packet, stores 1 to 8 coded-frame ranges, and enters
    `PacketActive`.
  - On invalid bitstream input it clears packet state and remains `Ready`.
- `decode_next()` is valid only in `PacketActive`.
  - It consumes exactly one coded frame.
  - It returns zero or one shown-frame descriptor.
  - After the final coded frame, it clears packet ownership and enters `Ready`.
  - After a core decode error, the paired JS binding discards the instance.

The target uses one wasm thread, no shared memory, and no imported JS callbacks
from decoder exports. Shared-memory workers or imported callbacks require either
per-worker instances or explicit state synchronization.

## Core API boundary

Consumer requirements:

- `vip9r-wasm` needs packet splitting, then exactly-one-coded-frame decode.
- Core unit tests use native host-backed storage, without wasm offsets or
  `memory.grow`.
- Host whole-packet decode wraps packet splitting and one-frame decode.

Target model:

- `vip9r_core::Decoder`: VP9 semantic session state such as reference metadata,
  probability contexts, segmentation state, and counters.
- `DecodeWorkspace`: a borrowed view over geometry-sized storage such as frame
  buffers, maps, and scratch.
- Native owned workspace: `std`/test convenience that owns host allocations and
  yields a `DecodeWorkspace`.
- `vip9r_wasm` state: wasm boundary state such as workspace layout, packet
  cursor, result block, and exported entry points.

`vip9r_core::Decoder` is not the wasm instance. It must not own wasm offsets,
packet staging, result blocks, or `memory.grow` policy. It also must not own
large frame/map buffers directly; those are supplied through the workspace.

Primary core operations:

- split one demuxed VP9 packet into 1 to 8 coded-frame ranges;
- apply VP9 semantic state transitions, including keyframe/intra-only state
  clearing;
- compute workspace requirements from decoder limits;
- decode one coded frame with a supplied workspace and return either no output
  or one shown frame.

Core output borrows are tied to the supplied workspace and are invalid after the
next decode or packet transition.

`FrameSink` is a packet-level adapter that may emit multiple shown frames. The
wasm step API emits at most one shown frame per call. Packet-level host decode
wraps packet splitting and one-frame decode.

Storage placement follows semantics, not byte count:

- `Decoder` may own fixed-size, dimension-independent VP9 session state.
- The workspace owns buffers whose size depends on instance limits or active
  frame geometry: pixels, MI-sized maps, row/column contexts, and large
  per-frame scratch.
- Immutable spec tables belong in ordinary read-only Rust data.

Do not use a hard byte threshold. The distinction is semantic session state
versus geometry-sized decode storage.

## VP9 session state

VP9 decode is not a pure `decode_frame(input, output)` operation. The decoder
must preserve state across packets:

- 8 reference-frame slots;
- per-reference metadata: decoded size, subsampling, bit depth;
- 4 persistent probability contexts plus the current working context;
- persistent segmentation feature state and segment maps;
- previous-frame MV/ref maps when `UsePrevFrameMvs` is allowed.

## Workspace sizing

Instance limits, not the current frame size, determine persistent capacity.
`max_width` and `max_height` are VP9 decoded sample dimensions:
`FrameWidth` and `FrameHeight`, the Y-plane dimensions parsed from the
bitstream. Render size, stride alignment, borders, padding, and SIMD-friendly
rounding are derived from those limits.

Worst simple pixel capacity for Profile 0 / 8-bit:

```text
i420_bytes(w, h) = w*h + 2*ceil(w/2)*ceil(h/2)
pixel_capacity = 9 * i420_bytes(max_width, max_height)
```

That is 8 reference slots plus one current reconstruction frame before stride,
alignment, padding, or implementation-private borders. Multiple reference slots
may alias the same decoded frame after refresh, but correctness must not depend
on aliasing.

Frame-size changes can happen mid-GOP. Active dimensions drive current decode
loops and output. Reference prediction and `show_existing_frame` use
per-reference dimensions. Allocation must reject dimensions above instance
limits before trusting bitstream-derived offsets or sizes.

Pixels are only one large allocation class. Metadata and scratch are also sized
from frame or MI dimensions:

```text
mi_w = ceil(width / 8)
mi_h = ceil(height / 8)
mi = mi_w * mi_h
```

Geometry-sized workspace requirements must account for:

- current and previous segmentation maps;
- current and previous MV/ref maps for inter prediction;
- mode-info needed after reconstruction by loop filtering: skip, transform size,
  block size, mode, reference frame, interpolation filter, and segment id;
- above/left syntax contexts that scale with frame width or active row;
- transform scratch;
- inter-prediction temporaries, using spec-derived conservative bounds until the
  implementation has tighter evidence.

Fixed decoder state and packet state include per-frame probability counts and
the packet/superframe cursor. They are not part of the geometry-sized workspace.

Exact byte layout belongs in `WorkspaceRequirements` and wasm workspace-layout
code, not in this design doc. Exact spec constants belong in implementation code
with nearby spec citations and tests, not as a second shadow copy here.

## Linear-memory ownership

JS can hold `Uint8Array` views onto wasm linear memory, but wasm can only
dereference its own memory. Decode internals are not JS-owned buffers.

Use a persistent workspace sized from instance limits:

```text
instance header/static state
reference frame pool, capacity = max_width/max_height
current frame, capacity = max_width/max_height
metadata maps, capacity = ceil(max_width/8) * ceil(max_height/8)
fixed scratch
packet staging tail, growable
```

The manually managed linear-memory region starts after linker-managed wasm
memory, currently using the linker-provided heap base aligned up to the largest
workspace-region alignment. Keep verifying the exported data/heap boundary when
link settings change.

`init(max_width, max_height)` lays out the fixed workspace once: reference
frames, current frame, metadata maps, and scratch first; the packet staging tail
starts after that fixed workspace. Changing `max_width` or `max_height` means
constructing a new `WebAssembly.Instance`.

Mid-GOP resize reshapes active views inside this capacity. Each frame recomputes
active dimensions, plane slices, MI rectangles, tile bounds, and per-slot
reference dimensions. It does not change the workspace layout.

This avoids mid-memory relocation. Wasm memory grows only at the end; if
reference frames or metadata maps were initially laid out for a smaller size,
later resizing would require relocation, abandoned old allocations, a real
allocator, or copying persistent decoder state. The fixed-capacity workspace
remains the default until measurements justify a smaller-footprint design.

`memory.grow` is not a substitute for resource limits. Use it only for the
packet staging tail in the minimal decoder path. Fixed workspace regions never
move or change capacity after `init`; only the tail capacity may increase.

`vip9r-wasm` has three storage domains:

- Static/stack wasm memory: control state, `Decoder`, small fixed tables, and
  the result block. No large frame-sized objects.
- Manually managed linear memory: fixed workspace regions for reference frames,
  the current frame, metadata maps, and large scratch, followed by the packet
  tail. Fixed ranges are computed by `init`; the packet tail keeps a fixed base
  with growable capacity.
- JS views: packet copy source/sink and output readers. JS gets offsets only,
  never ownership.

The wasm crate stays `no_std` and avoids `alloc` for the decoder path. Any
future allocator needs explicit ownership of its linear-memory region; it must
not independently consume the fixed workspace or packet tail.

Persistent wasm state stores ranges and typed layout descriptors, not Rust
slices. Slices are formed only inside an export after overflow, containment, and
alignment checks, and no `memory.grow` happens while a slice exists. Do not
build a `Decoder<'static>` containing workspace slices; that creates a
self-referential singleton and makes grow/lifetime reasoning worse.

## Packet tail

The packet tail is the only input pointer JS receives and the only manually
managed region that `reserve_input` may grow.

- `reserve_input(len)` checks/grows memory for the packet range, then writes the
  input pointer and capacity to the result block. It does not create a Rust
  slice.
- JS copies the complete demuxed VP9 packet into the reserved range.
- `begin_packet(len)` checks `len <= input_capacity`, forms a temporary
  immutable slice over that range, calls core packet splitting, stores up to 8
  coded-frame ranges relative to the packet base, then drops the slice.
- Until the packet is finished or the instance is dropped, the paired JS binding
  treats the packet range as decoder-owned and does not overwrite it.
- `decode_next()` forms a temporary slice for the current coded-frame range and
  temporary mutable views for workspace regions, then drops all Rust borrows
  before returning.

## Packet and step semantics

The wasm boundary requires one complete contiguous demuxed VP9 packet/superframe.
JS/demux owns network or WebM chunk accumulation. `begin_packet` passes the
reserved packet slice to core packet splitting, then stores coded-frame ranges
in wasm packet state.

`decode_next` consumes one coded frame from the active packet/superframe. A
coded frame can show at most one frame:

- hidden frame: no output;
- `show_existing_frame`: output one stored reference;
- normal decode: output the current frame only when `show_frame` is set.

One demuxed packet can contain a VP9 superframe. Superframe syntax stores
`frames_in_superframe_minus_1` in 3 bits, so one packet contains 1 to 8 coded
frames. The step API avoids an output queue: `begin_packet` establishes the
packet, and each `decode_next` returns synchronously with zero or one shown
frame descriptor.

`max_output_frames_per_packet = 8` is not JS/wasm output capacity. It is an
internal bound for superframe splitting and packet progress.

## Output descriptors

Shown-frame pixels live in the fixed workspace. The result block exposes the
decoder's native plane layout:

- decoded sample size;
- render size;
- Y/U/V plane offsets, lengths, and strides;
- current-packet coded-frame progress.

Plane byte lengths are allocated spans and may include row padding. Consumers
use the decoded/chroma width bytes at each stride.

The wasm boundary returns native plane descriptors. Compact I420 remains a copy
helper for tests and explicit copy-output adapters.

Returned descriptors and pointed-to pixel ranges are ephemeral. The caller must
consume or copy them before the next `decode_next`, `reserve_input`,
`begin_packet`, or instance teardown. This matches the target core output
lifetime and avoids queue/pinning/release APIs.

## Core/wasm responsibility split

- Core owns VP9 semantics, validation, workspace requirements, packet splitting,
  and typed workspace interpretation.
- Wasm owns linear-memory allocation, growth, workspace layout, packet cursor,
  exported entry points, result payloads, and resource limits.
- Native tests may use ordinary Rust allocations through core-owned test/helper
  workspace types.
- Tests that validate the wasm boundary itself - JS bindings and resource/layout
  edge cases - route through d8. Core golden tests may stay native.

The core must return resource-limit and invalid-layout errors before unsafe or
unchecked indexing.

`vip9r-core` currently forbids unsafe code. Keep VP9 semantic code safe by
default. If typed views over linear memory require unsafe code, concentrate it
in the wasm/workspace layer and guard it with explicit layout checks.

## Spec evidence

Local spec:
`docs/specs/vp9-bitstream-specification-v0.7-20170222-draft.md`.

- Reference slots: constants and overview, lines 378 and 808.
- `show_existing_frame`: header shortcut lines 1028-1035; output from reference
  slot lines 5068-5081.
- Reference update: per-slot metadata and sample copy, lines 5105-5112.
- Decode process order: loop filter before output/ref update, lines 3437-3447.
- Frame dimensions and MI sizing: lines 1167-1224.
- Mid-GOP resize/scaling: lines 843-845, 3051-3063, 3944-3952.
- Segmentation persistence: definition line 343; temporal update lookup lines
  2058-2111; clear behavior lines 2960-2965 and 3069-3079.
- Mode-info maps stored over 8x8 units: lines 1895-1913.
- Loop filter reads mode maps: lines 4738-4745 and 4817-4826.
- Previous MV/ref maps: lines 5113-5115; `UsePrevFrameMvs` conditions lines
  3069-3077.
- Above/left context bounds: lines 3197-3205.
- Superframes: lines 7812-7869.
- Per-block scratch: residual loop lines 2300-2352; reconstruct lines
  4327-4352; transform scratch lines 4362-4365; inter prediction temporaries
  lines 3818-3844 and 3971-4013.

Spec oddities to avoid baking into the boundary:

- output-process prose appears to transpose V-plane indexing;
- `SegmentId`/`SegmentIds` naming is inconsistent around resize clearing;
- `xStep/yStep <= 80` is looser than nearby scaling inequalities appear to
  imply.

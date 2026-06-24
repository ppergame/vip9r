# vip9r wasm ABI redesign

Target design for replacing `vip9r-wasm`'s placeholder ABI. This is an
implementation input, not a session log. Promote stable implemented decisions
into `docs/design.md`.

## Current baseline

- `vip9r-core` exposes stateful packet decode:
  `Decoder::decode_packet(packet, sink)`.
- Shown frames are pushed synchronously through `FrameSink`; frame borrows are
  valid only during the callback.
- That callback packet API is current implementation shape, not a wasm-boundary
  constraint.
- Public core output is compact I420 in libvpx-md5 order: visible Y, then U,
  then V. Internal storage may use stride or padding.
- `vip9r-wasm` still exports only placeholder helpers plus
  `vip9r_decode_frame(input_ptr, input_len, output_ptr, output_len)`, which
  ignores its pointers and returns `Unimplemented`.
- JS wasm bindings are empty.

## First ABI scope

One `WebAssembly.Instance` is one decoder session. The instance owns one
`vip9r_core::Decoder`, one fixed-capacity decode workspace, one packet cursor,
one transient packet staging range, and one integer result slot. Concurrent
decoders use separate wasm instances, not handles inside one shared linear
memory.

The wasm API accepts one complete contiguous demuxed VP9 packet/superframe,
splits it into coded-frame ranges, and advances one coded frame at a time. Each
step returns zero or one ephemeral shown-frame descriptor.

In scope for the first ABI:

- VP9 decoder session setup, reset, packet staging, packet splitting, and
  one-coded-frame stepping.
- Native plane descriptors for shown VP9 frames.
- Resource limits derived from `max_width` and `max_height`.
- d8/browser-callable integer ABI with manual JS bindings.

Out of scope for the first ABI:

- WebM/network accumulation and timestamp policy. JS owns both.
- Multiple decoder handles inside one wasm instance.
- JS-owned decode buffers.
- Output queues, frame pin/release APIs, and delayed VP9 frame retention for a
  later encoder stage.
- H.264/minih264 integration.
- Workers or shared wasm memory.

## Export surface

All mutating exports return an `i32` status. Payloads are written to
`ResultSlot`; status returns are not overloaded with pointers.

```text
vip9r_abi_version() -> u32
vip9r_result_ptr() -> u32

vip9r_init(max_width: u32, max_height: u32) -> i32
vip9r_reset() -> i32
vip9r_reserve_input(len: u32) -> i32
vip9r_begin_packet(len: u32) -> i32
vip9r_decode_next() -> i32
```

`vip9r_abi_version` returns `1` for this ABI. `vip9r_result_ptr` returns the
linear-memory offset of the per-instance `ResultSlot` static. Existing
placeholder helper exports may remain temporarily, but new callers use only the
exports above.

`reserve_input` stores the canonical input pointer and current input capacity in
`ResultSlot`. JS copies the complete packet to that range, then calls
`begin_packet(len)`. `begin_packet` never accepts arbitrary packet pointers.

## Status codes

Numeric status codes are ABI, so keep them stable after implementation.

| Code | Name | Meaning |
| ---: | --- | --- |
| 0 | `VIP9R_OK` | Export completed. Inspect `ResultSlot.flags` for output/progress. |
| -1 | `VIP9R_INVALID_CONFIG` | Invalid init limits or impossible static layout. |
| -2 | `VIP9R_OUTPUT_TOO_SMALL` | Reserved for copy-helper APIs. Not expected from the primary step ABI. |
| -3 | `VIP9R_RESOURCE_LIMIT` | Limit exceeded, checked arithmetic failed, or `memory.grow` failed. |
| -4 | `VIP9R_UNSUPPORTED_PROFILE` | Bitstream profile is outside profile 0 / 8-bit scope. |
| -5 | `VIP9R_UNSUPPORTED_BIT_DEPTH` | Bitstream bit depth is outside 8-bit scope. |
| -6 | `VIP9R_INVALID_BITSTREAM` | Malformed VP9 packet/frame. |
| -7 | `VIP9R_CALLBACK_ERROR` | Reserved to match current core `Sink` errors. Not returned by this ABI. |
| -8 | `VIP9R_UNIMPLEMENTED` | Implementation has reached an unfinished decoder block. |
| -9 | `VIP9R_INVALID_STATE` | Export is not valid in the current session phase. |
| -10 | `VIP9R_REENTRANT` | An export was entered while another export was active. |

Payload convention:

- `required_size` is set for `OUTPUT_TOO_SMALL` and for resource failures where
  the required byte count is known.
- `detail0` contains unsupported profile or bit depth when applicable.
- `detail0/detail1` contain actual width/height when a frame exceeds instance
  limits; `detail2/detail3` contain configured max width/height.
- Fields without useful payload are zero.

## Result slot

Use one fixed integer-only result struct in linker-managed wasm memory. Accessor
exports are not part of the first ABI; add them only with measured need.

```rust
#[repr(C)]
pub struct ResultSlot {
    pub abi_version: u32,
    pub status: i32,
    pub flags: u32,

    pub input_ptr: u32,
    pub input_capacity: u32,
    pub required_size: u32,

    pub packet_bytes: u32,
    pub packet_frames: u32,
    pub packet_index: u32,

    pub decoded_width: u32,
    pub decoded_height: u32,
    pub render_width: u32,
    pub render_height: u32,

    pub y_ptr: u32,
    pub y_len: u32,
    pub y_stride: u32,
    pub u_ptr: u32,
    pub u_len: u32,
    pub u_stride: u32,
    pub v_ptr: u32,
    pub v_len: u32,
    pub v_stride: u32,

    pub detail0: u32,
    pub detail1: u32,
    pub detail2: u32,
    pub detail3: u32,
}
```

`flags` bits:

```text
1 << 0  OUTPUT       shown-frame descriptor fields are valid
1 << 1  PACKET_DONE  the active packet is finished after this export
1 << 2  MEMORY_GREW  this export successfully grew wasm memory
```

Rules:

- Every export writes `status` and clears stale output fields before returning.
- `decode_next` sets `OUTPUT` only when a shown frame is available.
- `decode_next` sets `PACKET_DONE` on the step that consumes the final coded
  frame in the active packet.
- `packet_index` is the zero-based coded-frame index within the active packet.
- Plane lengths are allocated byte spans including row padding. Row consumers
  use the decoded/chroma width bytes at each stride.
- JS must reacquire `memory.buffer`, typed-array views, and the `ResultSlot`
  view after every successful `reserve_input`; `memory.grow` can detach old host
  views even though numeric wasm offsets remain stable.
- Global session counters are not part of the first ABI. JS already owns packet
  order and timestamp mapping; wasm exposes only current-packet progress.

## Session state machine

Phases:

```text
Uninit --init--> Ready --begin_packet--> PacketActive --packet_done--> Ready
Ready/PacketActive/Failed --reset--> Ready
PacketActive --decode_error--> Failed
```

Transitions:

- `init(max_width, max_height)` is valid only in `Uninit`.
  - On success it partitions the fixed workspace and enters `Ready`.
  - On invalid config/resource failure it stays `Uninit`.
  - Changing limits requires a new `WebAssembly.Instance`.
- `reset()` is valid in `Ready`, `PacketActive`, and `Failed`.
  - It clears VP9 decoder state, clears packet state, retains workspace
    capacity, and enters `Ready`.
  - It is invalid before a successful `init`.
- `reserve_input(len)` is valid only in `Ready`.
  - It grows only the packet tail if needed.
  - It does not create a Rust slice and does not change phase.
- `begin_packet(len)` is valid only in `Ready`.
  - It requires `len > 0` and `len <= input_capacity`.
  - It splits the packet, stores 1 to 8 coded-frame ranges, and enters
    `PacketActive`.
  - On error it clears packet state and remains `Ready`.
- `decode_next()` is valid only in `PacketActive`.
  - It consumes exactly one coded frame.
  - It returns zero or one shown-frame descriptor.
  - After the final coded frame, it clears packet ownership and enters `Ready`.
  - Any non-OK core decode status enters `Failed`; `reset` is then required
    before more input.

Every export rejects reentry with `VIP9R_REENTRANT`. The target uses one wasm
thread and no shared memory. If workers with shared wasm memory enter the
design, this state machine needs atomics or per-worker instances.

## Core API boundary

Drive the core/wasm API from actual consumers:

- `vip9r-wasm` needs packet splitting, then exactly-one-coded-frame decode.
- Core unit tests need native host-backed storage. They should not know about
  wasm offsets or `memory.grow`.
- Host whole-packet decode is a helper convenience, not the primary contract.

Target core model objects:

```text
vip9r_core::Decoder
  VP9 semantic session: reference metadata, probability contexts,
  segmentation state, counters

vip9r_core::DecodeWorkspace<'a>
  typed borrowed view over large storage: frame buffers, maps, scratch

vip9r_core::OwnedWorkspace
  std/test convenience that owns host allocations and yields DecodeWorkspace

vip9r_wasm::WasmState
  ABI state: arena placement, packet cursor, result slot, reentrancy guard
```

`vip9r_core::Decoder` is not the wasm instance. It must not own wasm offsets,
packet staging, result slots, or `memory.grow` policy. It also must not own
large frame/map buffers directly; those are supplied through `DecodeWorkspace`.

Primary target API shape:

```rust
pub fn split_packet(packet: &[u8]) -> Result<PacketFrames, DecodeError>;

pub struct PacketFrames {
    ranges: [ByteRange; 8],
    len: u8,
}

impl Decoder {
    pub fn new(options: DecoderOptions) -> Result<Self, DecodeError>;
    pub fn reset(&mut self);

    pub fn workspace_requirements(
        limits: DecoderLimits,
    ) -> Result<WorkspaceRequirements, DecodeError>;

    pub fn decode_frame<'w>(
        &mut self,
        coded_frame: &[u8],
        workspace: &'w mut DecodeWorkspace<'_>,
    ) -> Result<DecodeOutcome<'w>, DecodeError>;
}

pub enum DecodeOutcome<'a> {
    NoOutput,
    Output(ShownFrame<'a>),
}
```

The Rust surface may express lifetimes differently, but the invariant is fixed:
output borrows are tied to the supplied workspace and are invalid after the next
decode, reset, or packet transition.

`FrameSink` is not the primary boundary. It solves a packet-level API that may
emit multiple shown frames, while the wasm step API emits at most one shown
frame per call. Return `DecodeOutcome` for the primary core operation. If host
tooling needs packet-level decoding, implement it as a thin helper around
`split_packet` and `decode_frame`; do not make `decode_packet` a required core
primitive.

Storage placement follows semantics, not byte count:

- `Decoder` may own fixed-size, dimension-independent VP9 session state:
  probability contexts and other persistent semantic state.
- `DecodeWorkspace` owns buffers whose size depends on instance limits or
  active frame geometry: pixels, MI-sized maps, row/column contexts, and large
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

`reset()` clears decoder state but retains the instance workspace.

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

## Metadata and scratch

Pixels are only one large allocation class. Metadata is `O(mi)`:

```text
mi_w = ceil(width / 8)
mi_h = ceil(height / 8)
mi = mi_w * mi_h
```

Workspace requirements must account for:

- current and previous segmentation maps: conservative bound `2 * mi` entries;
- current and previous MV/ref maps for inter prediction;
- mode-info maps needed after reconstruction by loop filtering: skip, tx size,
  block size, mode, ref frame, interpolation filter, segment id;
- probability counts reset per coded frame;
- above/left syntax contexts that scale with frame width or the active row;
- superframe splitting state: at most 8 coded-frame entries and a 34-byte index;
- coefficient/dequant buffers: max 1024 entries for a 32x32 transform;
- inter-prediction temporary rows. Use a conservative private bound until
  measured otherwise; the spec's loose `xStep/yStep <= 80` permits a conceptual
  luma intermediate of `64 * 323` samples per reference.

The exact byte layout is internal to `WorkspaceRequirements` and `ArenaLayout`;
it is not exposed through the ABI.

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

Mid-GOP resize reshapes active views inside this capacity. Each frame recomputes
active dimensions, plane slices, MI rectangles, tile bounds, and per-slot
reference dimensions. It does not repartition the arena.

This avoids mid-memory relocation. Wasm memory grows only at the end; if
reference frames or metadata maps were initially partitioned for a smaller size,
later resizing would require relocation, abandoned old allocations, a real
allocator, or copying persistent decoder state. Do not implement that unless
measurement forces a smaller-footprint design.

`memory.grow` is not a substitute for resource limits. Use it only for the
packet staging tail in the minimal decoder path.

## Rust memory model inside `vip9r-wasm`

`vip9r-wasm` has three storage domains:

| Domain | Rust representation | Contents | Rule |
| --- | --- | --- | --- |
| Linker-managed static/stack memory | ordinary `static`s and stack locals | control state, `Decoder`, small fixed tables, `ResultSlot` | no large frame-sized objects |
| Manual wasm arena | numeric offsets into linear memory | reference frames, current frame, metadata maps, large scratch, packet tail | all ranges are computed by `init` |
| JS views | `Uint8Array` outside Rust | packet copy source/sink, output readers | JS gets offsets only, never ownership |

The wasm crate stays `no_std` and avoids `alloc` for the decoder path. A
general-purpose allocator would also want the linear-memory heap; mixing it with
a manual decode arena makes ownership unclear. If an allocator is later needed
for minih264 or tooling, give it an explicit region or make it the arena owner.
Do not let it independently consume `__heap_base`.

### Global control block

There is one mutable control block per `WebAssembly.Instance`. It is a Rust
`static`, not an arena allocation:

```rust
struct WasmState {
    phase: Phase,          // Uninit, Ready, PacketActive, Failed
    busy: bool,            // rejects accidental reentrant exports
    decoder: Option<Decoder>,
    arena: ArenaLayout,
    packet: PacketState,   // up to 8 coded-frame ranges
    result: ResultSlot,    // integer-only ABI payloads
}
```

Access is through a tiny `UnsafeCell` wrapper around the static. That is the
first unavoidable unsafe boundary: an exported C ABI function has to recover
`&mut WasmState` from static storage. Keep the safety argument narrow:

- target is one wasm thread and no shared memory;
- exports do not call imported JS callbacks;
- every export sets `busy` before mutating session state and clears it before
  returning;
- a reentrant call returns `VIP9R_REENTRANT` instead of creating a second mutable
  borrow.

### Arena base and growth

The manual arena starts at linker-provided `__heap_base`, aligned up to the
largest alignment required by typed regions. The current wasm artifact exports
`__data_end` and `__heap_base`; keep verifying that when link settings change.

Use wasm page operations directly:

```rust
const WASM_PAGE: usize = 64 * 1024;

unsafe extern "C" {
    static __heap_base: u8;
}

fn heap_base() -> usize {
    (&raw const __heap_base) as usize
}

fn memory_len() -> Result<usize, Status> {
    core::arch::wasm32::memory_size(0)
        .checked_mul(WASM_PAGE)
        .ok_or(Status::ResourceLimit)
}

fn ensure_memory(end: usize) -> Result<bool, Status> {
    let current = memory_len()?;
    if end <= current {
        return Ok(false);
    }
    let missing = end.checked_sub(current).ok_or(Status::ResourceLimit)?;
    let extra_pages = ceil_div(missing, WASM_PAGE)?;
    if core::arch::wasm32::memory_grow(0, extra_pages) == usize::MAX {
        return Err(Status::ResourceLimit);
    }
    Ok(true)
}
```

`init(max_width, max_height)` is the only operation that partitions the fixed
workspace:

```text
cursor = align_up(__heap_base, ARENA_ALIGN)
refs        = alloc_frame_pool(cursor, 8, max_width, max_height)
current     = alloc_frame(cursor, max_width, max_height)
metadata    = alloc_metadata(cursor, ceil(max_width/8) * ceil(max_height/8))
scratch     = alloc_scratch(cursor, max_width, max_height)
fixed_end   = cursor
packet_base = fixed_end
```

After `init`, `refs`, `current`, `metadata`, and `scratch` never move and never
change capacity. `packet_base` is also fixed, but its capacity is
`memory_len()? - packet_base`, so `reserve_input` can extend it with
`memory.grow`.

Changing `max_width` or `max_height` means constructing a new
`WebAssembly.Instance`.

### Ranges, not stored references

Persistent state stores ranges and typed layout descriptors, not Rust slices:

```rust
struct ByteRange {
    offset: u32,
    len: u32,
}

struct TypedRange<T> {
    offset: u32,
    len: u32, // element count
    _marker: PhantomData<T>,
}
```

Slices are formed only inside an export, after checking:

- `offset + byte_len` does not overflow;
- the range is inside the arena subrange that owns it;
- typed ranges are correctly aligned for `T`;
- no `memory.grow` happens while the slice exists.

The unsafe helper is mechanically small:

```rust
fn with_bytes_mut<R>(
    range: ByteRange,
    owner: ByteRange,
    f: impl FnOnce(&mut [u8]) -> R,
) -> Result<R, Status> {
    // checked arithmetic and containment first
    let len = checked_byte_len(range, owner)?;
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(range.offset as *mut u8, len)
    };
    Ok(f(bytes))
}
```

The slice lifetime is scoped to the closure. Do not build a `Decoder<'static>`
containing arena slices; that creates a self-referential singleton and makes
grow/lifetime reasoning worse.

Core-facing APIs therefore take a workspace/view argument for operations that
need large storage. Native tests can provide a `Vec`-backed `OwnedWorkspace`;
wasm provides a `DecodeWorkspace` view over the arena. The persistent `Decoder`
owns semantic state and reference-slot metadata, not the byte buffers
themselves.

### Packet tail

`packet_base` is the only input pointer JS receives.

- `reserve_input(len)` checks/grows memory for `packet_base + len`, writes
  `input_ptr`, `input_capacity`, and `required_size`, then returns. It does not
  create a Rust slice.
- JS copies the complete demuxed VP9 packet into
  `memory[packet_base..packet_base + len]`.
- `begin_packet(len)` checks `len <= input_capacity`, forms a temporary
  immutable slice over that range, calls `vip9r_core::split_packet`, stores up
  to 8 coded-frame `ByteRange`s relative to `packet_base`, then drops the
  slice.
- Until the packet is finished or `reset` is called, JS must treat the packet
  range as decoder-owned and must not overwrite it.
- `decode_next()` forms a temporary slice for the current coded-frame range and
  temporary mutable views for workspace regions, then drops all Rust borrows
  before returning.

## Packet and step semantics

The wasm API requires one complete contiguous demuxed VP9 packet/superframe.
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

Shown-frame pixels live in the fixed arena. `ResultSlot` exposes the decoder's
native plane layout:

- decoded sample size;
- render size;
- Y/U/V plane offsets, lengths, and strides;
- current-packet coded-frame progress.

Compact I420 remains a copy helper for tests and simple callers, not the
primary wasm output descriptor.

Returned descriptors and pointed-to pixel ranges are ephemeral. The caller must
consume or copy them before the next `decode_next`, `reserve_input`,
`begin_packet`, `reset`, or instance teardown. This matches the target core
`DecodeOutcome` lifetime and avoids queue/pinning/release APIs.

## Core/wasm responsibility split

- Core owns VP9 semantics, validation, workspace requirements, packet splitting,
  and typed workspace interpretation.
- Wasm owns linear-memory allocation, growth, arena placement, packet cursor,
  exported entry points, ABI result fields, and resource limits.
- Native tests may use ordinary Rust allocations through core-owned test/helper
  workspace types.
- Tests that validate the wasm boundary itself - JS bindings, adversarial
  resource/layout cases, and fuzz-style boundary inputs - route through d8.
  Ordinary core golden tests may stay native.

The core must return resource-limit and invalid-layout errors before unsafe or
unchecked indexing. It does not need to be the sandbox.

`vip9r-core` currently forbids unsafe code. Keep VP9 semantic code safe by
default. If packed wasm arenas or typed views over linear memory require unsafe
code, concentrate it in the wasm/workspace layer and guard it with explicit
layout checks.

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
  2058-2111; reset/clear behavior lines 2960-2965 and 3069-3079.
- Mode-info maps stored over 8x8 units: lines 1895-1913.
- Loop filter reads mode maps: lines 4738-4745 and 4817-4826.
- Previous MV/ref maps: lines 5113-5115; `UsePrevFrameMvs` conditions lines
  3069-3077.
- Above/left context bounds: lines 3197-3205.
- Superframes: lines 7812-7869.
- Per-block scratch: residual loop lines 2300-2352; reconstruct lines
  4327-4352; transform scratch lines 4362-4365; inter prediction temporaries
  lines 3818-3844 and 3971-4013.

Spec oddities to avoid baking into the ABI:

- output-process prose appears to transpose V-plane indexing;
- `SegmentId`/`SegmentIds` naming is inconsistent around resize clearing;
- `xStep/yStep <= 80` is looser than nearby scaling inequalities appear to
  imply.

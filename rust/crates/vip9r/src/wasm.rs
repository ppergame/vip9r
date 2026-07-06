use core::{
    fmt::{self, Write},
    panic::PanicInfo,
};

use crate::{
    DecodeError, DecodeOutcome, DecodeWorkspace, Decoder, I420Frame, Plane, WorkspaceLayout,
    split_packet,
};

const OK: i32 = 0;
const INVALID_STATE: i32 = -9;
const RESOURCE_LIMIT: i32 = -3;
const WASM_PAGE: usize = 64 * 1024;
const ARENA_ALIGN: usize = 16;
// Threads (M6): 4 threads total, fixed-address shadow stacks. The coordinator
// keeps the linker-default stack-first stack (0..1 MiB, size pinned by
// -zstack-size in rust/.cargo/config.toml; overflow wraps below zero and
// traps). The three workers bind the fixed 1 MiB regions at 1..4 MiB — tops
// 0x200000 / 0x300000 / 0x400000 are ABI constants JS writes to a fresh
// instance's exported `__stack_pointer` global, executing no wasm —
// and --global-base pushes the data section to 4 MiB to keep the region
// clear. Worker stacks have no overflow trap: overflow walks down into the
// neighboring stack. arena_bounds asserts the linker honored the layout.
const WORKER_STACKS_END: usize = 4 << 20;
const MAX_CODED_FRAMES: usize = crate::MAX_CODED_FRAMES_PER_PACKET;
const LOG_BUFFER_LEN: usize = 1024;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    log(LogKind::Panic, format_args!("{info}"));
    core::arch::wasm32::unreachable();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LogKind {
    Diagnostic = 0,
    TestFailure = 1,
    Panic = 2,
}

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn vip9r_log(kind: i32, ptr: *const u8, len: usize);
}

struct LogBuffer {
    bytes: [u8; LOG_BUFFER_LEN],
    len: usize,
}

impl LogBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; LOG_BUFFER_LEN],
            len: 0,
        }
    }
}

impl Write for LogBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = self.bytes.len() - self.len;
        let mut copy_len = remaining.min(s.len());
        while !s.is_char_boundary(copy_len) {
            copy_len -= 1;
        }

        self.bytes[self.len..self.len + copy_len].copy_from_slice(&s.as_bytes()[..copy_len]);
        self.len += copy_len;
        if copy_len == s.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

pub(crate) fn log(kind: LogKind, args: fmt::Arguments<'_>) {
    let mut buffer = LogBuffer::new();
    let _ = fmt::write(&mut buffer, args);
    unsafe { vip9r_log(kind as i32, buffer.bytes.as_ptr(), buffer.len) };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResultBlock {
    input_ptr: u32,
    input_capacity: u32,

    packet_len: u32,
    coded_frame_count: u32,
    coded_frame_index: u32,

    has_output: u32,
    packet_done: u32,

    decoded_width: u32,
    decoded_height: u32,
    render_width: u32,
    render_height: u32,

    y_ptr: u32,
    y_len: u32,
    y_stride: u32,

    u_ptr: u32,
    u_len: u32,
    u_stride: u32,

    v_ptr: u32,
    v_len: u32,
    v_stride: u32,
}

impl ResultBlock {
    const fn new() -> Self {
        Self {
            input_ptr: 0,
            input_capacity: 0,
            packet_len: 0,
            coded_frame_count: 0,
            coded_frame_index: 0,
            has_output: 0,
            packet_done: 0,
            decoded_width: 0,
            decoded_height: 0,
            render_width: 0,
            render_height: 0,
            y_ptr: 0,
            y_len: 0,
            y_stride: 0,
            u_ptr: 0,
            u_len: 0,
            u_stride: 0,
            v_ptr: 0,
            v_len: 0,
            v_stride: 0,
        }
    }

    fn clear_output(&mut self) {
        self.has_output = 0;
        self.decoded_width = 0;
        self.decoded_height = 0;
        self.render_width = 0;
        self.render_height = 0;
        self.y_ptr = 0;
        self.y_len = 0;
        self.y_stride = 0;
        self.u_ptr = 0;
        self.u_len = 0;
        self.u_stride = 0;
        self.v_ptr = 0;
        self.v_len = 0;
        self.v_stride = 0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Uninit,
    Ready,
    PacketActive,
    Poisoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Region {
    offset: u32,
    len: u32,
}

impl Region {
    const fn empty() -> Self {
        Self { offset: 0, len: 0 }
    }

    fn end(self) -> Result<u32, i32> {
        self.offset.checked_add(self.len).ok_or(RESOURCE_LIMIT)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivePacket {
    len: u32,
    frames: [Region; MAX_CODED_FRAMES],
    frame_count: u32,
    next_index: u32,
}

impl ActivePacket {
    const fn new() -> Self {
        Self {
            len: 0,
            frames: [Region::empty(); MAX_CODED_FRAMES],
            frame_count: 0,
            next_index: 0,
        }
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

#[derive(Debug)]
struct Session {
    phase: Phase,
    decoder: Option<Decoder>,
    layout: Option<WorkspaceLayout>,
    workspace: Region,
    input_base: u32,
    input_capacity: u32,
    packet: ActivePacket,
    result: ResultBlock,
}

impl Session {
    const fn new() -> Self {
        Self {
            phase: Phase::Uninit,
            decoder: None,
            layout: None,
            workspace: Region::empty(),
            input_base: 0,
            input_capacity: 0,
            packet: ActivePacket::new(),
            result: ResultBlock::new(),
        }
    }

    fn init(&mut self, max_width: u32, max_height: u32) -> Result<(), i32> {
        if self.phase != Phase::Uninit {
            return Err(INVALID_STATE);
        }

        let layout = WorkspaceLayout::new(max_width, max_height).map_err(|err| err.code())?;
        let decoder = Decoder::new(layout);

        let (workspace_base, input_base) = arena_bounds(layout)?;
        let workspace_len = layout.total_bytes();
        ensure_memory(input_base)?;
        let memory_len = memory_len()?;
        if input_base > memory_len {
            return Err(RESOURCE_LIMIT);
        }

        let workspace = Region {
            offset: u32::try_from(workspace_base).map_err(|_| RESOURCE_LIMIT)?,
            len: u32::try_from(workspace_len).map_err(|_| RESOURCE_LIMIT)?,
        };
        let input_capacity = u32::try_from(memory_len - input_base).map_err(|_| RESOURCE_LIMIT)?;
        let input_base = u32::try_from(input_base).map_err(|_| RESOURCE_LIMIT)?;

        self.decoder = Some(decoder);
        self.layout = Some(layout);
        self.workspace = workspace;
        self.input_base = input_base;
        self.input_capacity = input_capacity;
        self.packet.clear();
        self.result = ResultBlock::new();
        self.result.input_ptr = self.input_base;
        self.result.input_capacity = self.input_capacity;
        self.phase = Phase::Ready;
        Ok(())
    }

    fn reserve_input(&mut self, len: u32) -> Result<(), i32> {
        if self.phase != Phase::Ready {
            return Err(INVALID_STATE);
        }

        let base = usize::try_from(self.input_base).map_err(|_| RESOURCE_LIMIT)?;
        let len = usize::try_from(len).map_err(|_| RESOURCE_LIMIT)?;
        let end = base.checked_add(len).ok_or(RESOURCE_LIMIT)?;
        ensure_memory(end)?;

        let memory_len = memory_len()?;
        self.input_capacity = memory_len
            .checked_sub(base)
            .and_then(|capacity| u32::try_from(capacity).ok())
            .ok_or(RESOURCE_LIMIT)?;

        self.result.input_ptr = self.input_base;
        self.result.input_capacity = self.input_capacity;
        Ok(())
    }

    fn begin_packet(&mut self, len: u32) -> Result<(), i32> {
        if self.phase != Phase::Ready {
            return Err(INVALID_STATE);
        }
        if len == 0 || len > self.input_capacity {
            return Err(DecodeError::InvalidBitstream.code());
        }

        let packet = bytes(Region {
            offset: self.input_base,
            len,
        })?;
        let frames = split_packet(packet).map_err(|err| {
            self.packet.clear();
            err.code()
        })?;

        let mut active = ActivePacket::new();
        active.len = len;
        active.frame_count = u32::try_from(frames.len()).map_err(|_| RESOURCE_LIMIT)?;
        for (index, frame) in frames.as_slice().iter().copied().enumerate() {
            active.frames[index] = range_from_packet_frame(self.input_base, frame)?;
        }

        self.packet = active;
        self.result.packet_len = len;
        self.result.coded_frame_count = active.frame_count;
        self.result.coded_frame_index = 0;
        self.result.has_output = 0;
        self.result.packet_done = 0;
        self.phase = Phase::PacketActive;
        Ok(())
    }

    fn decode_next(&mut self) -> Result<(), i32> {
        if self.phase != Phase::PacketActive {
            return Err(INVALID_STATE);
        }
        if self.packet.next_index >= self.packet.frame_count {
            return Err(INVALID_STATE);
        }

        self.result.clear_output();
        self.result.packet_done = 0;
        self.result.coded_frame_index = self.packet.next_index;

        let frame_range = self.packet.frames[self.packet.next_index as usize];
        let coded_frame = bytes(frame_range)?;

        let decoder = self.decoder.as_mut().ok_or(INVALID_STATE)?;
        let layout = self.layout.ok_or(INVALID_STATE)?;
        let workspace_bytes = bytes_mut(self.workspace)?;
        let mut workspace =
            DecodeWorkspace::new(layout, workspace_bytes).map_err(|err| err.code())?;
        let outcome = match decoder.decode_coded_frame(coded_frame, &mut workspace) {
            Ok(outcome) => outcome,
            Err(err) => {
                self.phase = Phase::Poisoned;
                return Err(err.code());
            }
        };

        self.packet.next_index += 1;
        if self.packet.next_index == self.packet.frame_count {
            self.result.packet_done = 1;
            self.phase = Phase::Ready;
            self.packet.clear();
        }

        match outcome {
            DecodeOutcome::NoOutput => Ok(()),
            DecodeOutcome::Output(frame) => self.write_frame(frame),
        }
    }

    fn write_frame(&mut self, frame: I420Frame<'_>) -> Result<(), i32> {
        let y = plane_descriptor(frame.y)?;
        let u = plane_descriptor(frame.u)?;
        let v = plane_descriptor(frame.v)?;
        let y_stride = u32::try_from(frame.y.shape.stride).map_err(|_| RESOURCE_LIMIT)?;
        let u_stride = u32::try_from(frame.u.shape.stride).map_err(|_| RESOURCE_LIMIT)?;
        let v_stride = u32::try_from(frame.v.shape.stride).map_err(|_| RESOURCE_LIMIT)?;

        self.result.has_output = 1;
        self.result.decoded_width = frame.info.visible_width;
        self.result.decoded_height = frame.info.visible_height;
        self.result.render_width = frame.info.render_width;
        self.result.render_height = frame.info.render_height;
        self.result.y_ptr = y.offset;
        self.result.y_len = y.len;
        self.result.y_stride = y_stride;
        self.result.u_ptr = u.offset;
        self.result.u_len = u.len;
        self.result.u_stride = u_stride;
        self.result.v_ptr = v.offset;
        self.result.v_len = v.len;
        self.result.v_stride = v_stride;
        Ok(())
    }
}

static mut SESSION: Session = Session::new();

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_result_ptr() -> u32 {
    let session = session();
    (&raw const session.result) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_init(max_width: u32, max_height: u32) -> i32 {
    status(session().init(max_width, max_height))
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_reserve_input(len: u32) -> i32 {
    status(session().reserve_input(len))
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_begin_packet(len: u32) -> i32 {
    status(session().begin_packet(len))
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_decode_next() -> i32 {
    status(session().decode_next())
}

// Worker thread entry (M6): a fresh instance over the shared memory parks
// here after JS rebinds its shadow stack. Workers never touch SESSION.
#[unsafe(no_mangle)]
pub extern "C" fn vip9r_worker_main(worker_index: u32) -> ! {
    crate::pool::worker_main(worker_index)
}

// The frontend calls this once after spawning all three pool workers over
// this memory; without it decode stays serial and dispatch asserts. The pool
// is all-or-nothing (join counts acknowledgements from every worker), so
// there is no worker-count parameter.
#[unsafe(no_mangle)]
pub extern "C" fn vip9r_pool_activate() {
    crate::pool::activate();
}

// Exact static requirement in wasm pages (shadow stacks + data + workspace
// arena) for a session with the given max dimensions; the packet tail sits
// above it, so a session's memory must add packet capacity on top. Pure.
// Memory is pinned at link time, so JS no longer calls this to size it; the
// export remains the dynamic-sizing ground truth, exercised by the
// corpus_ceiling_fits_fixed_memory test against the pinned size. Negative
// error code if the dimensions are rejected.
#[unsafe(no_mangle)]
pub extern "C" fn vip9r_required_pages(max_width: u32, max_height: u32) -> i32 {
    match required_pages(max_width, max_height) {
        Ok(pages) => pages,
        Err(code) => code,
    }
}

fn required_pages(max_width: u32, max_height: u32) -> Result<i32, i32> {
    let layout = WorkspaceLayout::new(max_width, max_height).map_err(|err| err.code())?;
    let (_, input_base) = arena_bounds(layout)?;
    i32::try_from(input_base.div_ceil(WASM_PAGE)).map_err(|_| RESOURCE_LIMIT)
}

fn arena_bounds(layout: WorkspaceLayout) -> Result<(usize, usize), i32> {
    assert!(
        heap_base() >= WORKER_STACKS_END,
        "data section overlaps the worker stack region: --global-base flag missing"
    );
    let workspace_base = align_up(heap_base(), ARENA_ALIGN)?;
    let workspace_end = workspace_base
        .checked_add(layout.total_bytes())
        .ok_or(RESOURCE_LIMIT)?;
    let input_base = align_up(workspace_end, ARENA_ALIGN)?;
    Ok((workspace_base, input_base))
}

fn status(result: Result<(), i32>) -> i32 {
    match result {
        Ok(()) => OK,
        Err(code) => code,
    }
}

fn session() -> &'static mut Session {
    unsafe { &mut *core::ptr::addr_of_mut!(SESSION) }
}

fn range_from_packet_frame(packet_base: u32, frame: crate::CodedFrameRange) -> Result<Region, i32> {
    let start = u32::try_from(frame.start).map_err(|_| RESOURCE_LIMIT)?;
    let len = u32::try_from(frame.len).map_err(|_| RESOURCE_LIMIT)?;
    let offset = packet_base.checked_add(start).ok_or(RESOURCE_LIMIT)?;
    Ok(Region { offset, len })
}

fn plane_descriptor(plane: Plane<'_>) -> Result<Region, i32> {
    let ptr = plane.data.as_ptr() as usize;
    let offset = u32::try_from(ptr).map_err(|_| RESOURCE_LIMIT)?;
    let len = u32::try_from(plane.data.len()).map_err(|_| RESOURCE_LIMIT)?;
    Ok(Region { offset, len })
}

fn align_up(value: usize, align: usize) -> Result<usize, i32> {
    let mask = align.checked_sub(1).ok_or(RESOURCE_LIMIT)?;
    value
        .checked_add(mask)
        .map(|v| v & !mask)
        .ok_or(RESOURCE_LIMIT)
}

fn bytes(region: Region) -> Result<&'static [u8], i32> {
    let end = region.end()?;
    let end = usize::try_from(end).map_err(|_| RESOURCE_LIMIT)?;
    if end > memory_len()? {
        return Err(RESOURCE_LIMIT);
    }
    let ptr = region.offset as *const u8;
    let len = usize::try_from(region.len).map_err(|_| RESOURCE_LIMIT)?;
    Ok(unsafe { core::slice::from_raw_parts(ptr, len) })
}

fn bytes_mut(region: Region) -> Result<&'static mut [u8], i32> {
    let end = region.end()?;
    let end = usize::try_from(end).map_err(|_| RESOURCE_LIMIT)?;
    if end > memory_len()? {
        return Err(RESOURCE_LIMIT);
    }
    let ptr = region.offset as *mut u8;
    let len = usize::try_from(region.len).map_err(|_| RESOURCE_LIMIT)?;
    Ok(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
}

unsafe extern "C" {
    static __heap_base: u8;
}

fn heap_base() -> usize {
    (&raw const __heap_base) as usize
}

fn memory_len() -> Result<usize, i32> {
    core::arch::wasm32::memory_size(0)
        .checked_mul(WASM_PAGE)
        .ok_or(RESOURCE_LIMIT)
}

fn ensure_memory(end: usize) -> Result<(), i32> {
    let current = memory_len()?;
    if end <= current {
        return Ok(());
    }

    let missing = end.checked_sub(current).ok_or(RESOURCE_LIMIT)?;
    let extra_pages = missing.div_ceil(WASM_PAGE);
    if core::arch::wasm32::memory_grow(0, extra_pages) == usize::MAX {
        return Err(RESOURCE_LIMIT);
    }
    Ok(())
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{WASM_PAGE, required_pages};

    // Memory is pinned at link time (--initial-memory == --max-memory in
    // rust/.cargo/config.toml) while this module keeps sizing dynamically;
    // the test runner instantiates over a memory of exactly the link-time
    // size. Pin the ceiling: the corpus maximum (1080p) plus the 8 MiB
    // packet-tail budget (anchors in createVip9rMemory, wasm-env.ts) must
    // fit, otherwise large sessions fail with RESOURCE_LIMIT at init on
    // devices instead of here.
    #[test]
    fn corpus_ceiling_fits_fixed_memory() {
        let tail_pages = (8 << 20) / WASM_PAGE;
        let required = required_pages(1920, 1080).unwrap();
        let required = usize::try_from(required).unwrap();

        assert!(required + tail_pages <= core::arch::wasm32::memory_size(0));
    }
}

use core::panic::PanicInfo;

use crate::{
    DecodeError, DecodeOutcome, DecodeWorkspace, Decoder, I420Frame, Plane, WorkspaceLayout,
    split_packet,
};

const OK: i32 = 0;
const INVALID_STATE: i32 = -9;
const RESOURCE_LIMIT: i32 = -3;
#[cfg(target_arch = "wasm32")]
const WASM_PAGE: usize = 64 * 1024;
const ARENA_ALIGN: usize = 16;
const MAX_CODED_FRAMES: usize = crate::MAX_CODED_FRAMES_PER_PACKET;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
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

    fn init(&mut self, max_width: u32, max_height: u32) -> i32 {
        if self.phase != Phase::Uninit {
            return INVALID_STATE;
        }

        let layout = match WorkspaceLayout::new(max_width, max_height) {
            Ok(layout) => layout,
            Err(err) => return err.code(),
        };
        let decoder = Decoder::new(layout);

        let workspace_base = match heap_base().and_then(|base| align_up(base, ARENA_ALIGN)) {
            Ok(base) => base,
            Err(code) => return code,
        };
        let workspace_len = layout.total_bytes();
        let workspace_end = match workspace_base.checked_add(workspace_len) {
            Some(end) => end,
            None => return RESOURCE_LIMIT,
        };
        let input_base = match align_up(workspace_end, ARENA_ALIGN) {
            Ok(base) => base,
            Err(code) => return code,
        };
        if let Err(code) = ensure_memory(input_base) {
            return code;
        }
        let memory_len = match memory_len() {
            Ok(len) => len,
            Err(code) => return code,
        };
        if input_base > memory_len {
            return RESOURCE_LIMIT;
        }

        let workspace_base_u32 = match u32::try_from(workspace_base) {
            Ok(base) => base,
            Err(_) => return RESOURCE_LIMIT,
        };
        let workspace_len_u32 = match u32::try_from(workspace_len) {
            Ok(len) => len,
            Err(_) => return RESOURCE_LIMIT,
        };
        let input_base_u32 = match u32::try_from(input_base) {
            Ok(base) => base,
            Err(_) => return RESOURCE_LIMIT,
        };
        let input_capacity = match u32::try_from(memory_len - input_base) {
            Ok(capacity) => capacity,
            Err(_) => return RESOURCE_LIMIT,
        };

        self.decoder = Some(decoder);
        self.layout = Some(layout);
        self.workspace = Region {
            offset: workspace_base_u32,
            len: workspace_len_u32,
        };
        self.input_base = input_base_u32;
        self.input_capacity = input_capacity;
        self.packet.clear();
        self.result = ResultBlock::new();
        self.result.input_ptr = self.input_base;
        self.result.input_capacity = self.input_capacity;
        self.phase = Phase::Ready;
        OK
    }

    fn reserve_input(&mut self, len: u32) -> i32 {
        if self.phase != Phase::Ready {
            return INVALID_STATE;
        }

        let end = match usize::try_from(self.input_base).ok().and_then(|base| {
            usize::try_from(len)
                .ok()
                .and_then(|len| base.checked_add(len))
        }) {
            Some(end) => end,
            None => return RESOURCE_LIMIT,
        };
        if let Err(code) = ensure_memory(end) {
            return code;
        }

        let memory_len = match memory_len() {
            Ok(len) => len,
            Err(code) => return code,
        };
        let base = match usize::try_from(self.input_base) {
            Ok(base) => base,
            Err(_) => return RESOURCE_LIMIT,
        };
        self.input_capacity = match memory_len
            .checked_sub(base)
            .and_then(|capacity| u32::try_from(capacity).ok())
        {
            Some(capacity) => capacity,
            None => return RESOURCE_LIMIT,
        };

        self.result.input_ptr = self.input_base;
        self.result.input_capacity = self.input_capacity;
        OK
    }

    fn begin_packet(&mut self, len: u32) -> i32 {
        if self.phase != Phase::Ready {
            return INVALID_STATE;
        }
        if len == 0 || len > self.input_capacity {
            return DecodeError::InvalidBitstream.code();
        }

        let packet = match bytes(Region {
            offset: self.input_base,
            len,
        }) {
            Ok(packet) => packet,
            Err(code) => return code,
        };
        let frames = match split_packet(packet) {
            Ok(frames) => frames,
            Err(err) => {
                self.packet.clear();
                return err.code();
            }
        };

        let mut active = ActivePacket::new();
        active.len = len;
        active.frame_count = match u32::try_from(frames.len()) {
            Ok(len) => len,
            Err(_) => return RESOURCE_LIMIT,
        };
        for (index, frame) in frames.as_slice().iter().copied().enumerate() {
            active.frames[index] = match range_from_packet_frame(self.input_base, frame) {
                Ok(range) => range,
                Err(code) => return code,
            };
        }

        self.packet = active;
        self.result.packet_len = len;
        self.result.coded_frame_count = active.frame_count;
        self.result.coded_frame_index = 0;
        self.result.has_output = 0;
        self.result.packet_done = 0;
        self.phase = Phase::PacketActive;
        OK
    }

    fn decode_next(&mut self) -> i32 {
        if self.phase != Phase::PacketActive {
            return INVALID_STATE;
        }
        if self.packet.next_index >= self.packet.frame_count {
            return INVALID_STATE;
        }

        self.result.clear_output();
        self.result.packet_done = 0;
        self.result.coded_frame_index = self.packet.next_index;

        let frame_range = self.packet.frames[self.packet.next_index as usize];
        let coded_frame = match bytes(frame_range) {
            Ok(coded_frame) => coded_frame,
            Err(code) => return code,
        };

        let Some(decoder) = self.decoder.as_mut() else {
            return INVALID_STATE;
        };
        let Some(layout) = self.layout else {
            return INVALID_STATE;
        };
        let workspace_bytes = match bytes_mut(self.workspace) {
            Ok(bytes) => bytes,
            Err(code) => return code,
        };
        let mut workspace = match DecodeWorkspace::new(layout, workspace_bytes) {
            Ok(workspace) => workspace,
            Err(err) => return err.code(),
        };
        let outcome = match decoder.decode_coded_frame(coded_frame, &mut workspace) {
            Ok(outcome) => outcome,
            Err(err) => {
                self.phase = Phase::Poisoned;
                return err.code();
            }
        };

        self.packet.next_index += 1;
        if self.packet.next_index == self.packet.frame_count {
            self.result.packet_done = 1;
            self.phase = Phase::Ready;
            self.packet.clear();
        }

        match outcome {
            DecodeOutcome::NoOutput => OK,
            DecodeOutcome::Output(frame) => self.write_frame(frame),
        }
    }

    fn write_frame(&mut self, frame: I420Frame<'_>) -> i32 {
        let y = match plane_descriptor(frame.y) {
            Ok(plane) => plane,
            Err(code) => return code,
        };
        let u = match plane_descriptor(frame.u) {
            Ok(plane) => plane,
            Err(code) => return code,
        };
        let v = match plane_descriptor(frame.v) {
            Ok(plane) => plane,
            Err(code) => return code,
        };

        self.result.has_output = 1;
        self.result.decoded_width = frame.info.visible_width;
        self.result.decoded_height = frame.info.visible_height;
        self.result.render_width = frame.info.render_width;
        self.result.render_height = frame.info.render_height;
        self.result.y_ptr = y.offset;
        self.result.y_len = y.len;
        self.result.y_stride = match u32::try_from(frame.y.shape.stride) {
            Ok(stride) => stride,
            Err(_) => return RESOURCE_LIMIT,
        };
        self.result.u_ptr = u.offset;
        self.result.u_len = u.len;
        self.result.u_stride = match u32::try_from(frame.u.shape.stride) {
            Ok(stride) => stride,
            Err(_) => return RESOURCE_LIMIT,
        };
        self.result.v_ptr = v.offset;
        self.result.v_len = v.len;
        self.result.v_stride = match u32::try_from(frame.v.shape.stride) {
            Ok(stride) => stride,
            Err(_) => return RESOURCE_LIMIT,
        };
        OK
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
    session().init(max_width, max_height)
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_reserve_input(len: u32) -> i32 {
    session().reserve_input(len)
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_begin_packet(len: u32) -> i32 {
    session().begin_packet(len)
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_decode_next() -> i32 {
    session().decode_next()
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

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    static __heap_base: u8;
}

#[cfg(target_arch = "wasm32")]
fn heap_base() -> Result<usize, i32> {
    Ok((&raw const __heap_base) as usize)
}

#[cfg(not(target_arch = "wasm32"))]
fn heap_base() -> Result<usize, i32> {
    Err(RESOURCE_LIMIT)
}

#[cfg(target_arch = "wasm32")]
fn memory_len() -> Result<usize, i32> {
    core::arch::wasm32::memory_size(0)
        .checked_mul(WASM_PAGE)
        .ok_or(RESOURCE_LIMIT)
}

#[cfg(not(target_arch = "wasm32"))]
fn memory_len() -> Result<usize, i32> {
    Err(RESOURCE_LIMIT)
}

#[cfg(target_arch = "wasm32")]
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

#[cfg(not(target_arch = "wasm32"))]
fn ensure_memory(_end: usize) -> Result<(), i32> {
    Err(RESOURCE_LIMIT)
}

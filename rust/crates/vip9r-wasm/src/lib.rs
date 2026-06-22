#![cfg_attr(target_arch = "wasm32", no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_arch = "wasm32")]
use core::panic::PanicInfo;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_abi_version() -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_required_i420_len(width: u32, height: u32) -> usize {
    vip9r_core::required_i420_len(width, height).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_required_yuv420_len(width: u32, height: u32) -> usize {
    vip9r_required_i420_len(width, height)
}

#[unsafe(no_mangle)]
pub extern "C" fn vip9r_decode_frame(
    input_ptr: *const u8,
    input_len: usize,
    output_ptr: *mut u8,
    output_len: usize,
) -> i32 {
    let _ = (input_ptr, input_len, output_ptr, output_len);
    vip9r_core::DecodeError::<core::convert::Infallible>::Unimplemented.code()
}

# Rust wasm32 intrinsics

Rust source: `1.96.0` commit `ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96`.

Coverage: public `core::arch::wasm32` functions and public aliases in the pinned `stdarch` wasm32 source files. Operator names come from `assert_instr` and `doc(alias)` attributes in the Rust source. Rustdoc links are stripped to plain text; code blocks, headings, bullets, and paragraphs from source doc comments are preserved.

## Module export gates

| Module | Public export status | Target feature on functions |
| --- | --- | --- |
| `memory.rs` | stable 1.33.0 (`simd_wasm32`) | none |
| `simd128.rs` | stable 1.54.0 (`wasm_simd`) | `simd128` |
| `relaxed_simd.rs` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) | `relaxed-simd` |
| `atomic.rs` | unstable (`stdarch_wasm_atomic_wait`, issue 77839) | `atomics` |
| `mod.rs` scalar/control/numeric functions | mixed; see rows | none |

## Types

- `v128`: Rust wasm-specific 128-bit SIMD vector type. The source states it corresponds to WebAssembly `v128`; the same type is used for i8x16/u8x16, i16x8/u16x8, i32x4/u32x4, i64x2/u64x2, f32x4, and f64x2 interpretations.

## Scalar/control/numeric (`mod.rs`)

### `unreachable`

- Signature: `pub fn unreachable() -> !`
- Wasm op / alias: `unreachable`
- Feature: -
- Status: stable 1.37.0 (`unreachable_wasm32`)
- Flags / constraints: -

Source documentation:

Generates the `unreachable` instruction, which causes an unconditional trap.

This function is safe to call and immediately aborts the execution.

### `f32_ceil`

- Signature: `pub fn f32_ceil(a: f32) -> f32`
- Wasm op / alias: `f32.ceil`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f32.ceil` instruction, returning the smallest integer greater than or equal to `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f32::ceil()`.

### `f32_floor`

- Signature: `pub fn f32_floor(a: f32) -> f32`
- Wasm op / alias: `f32.floor`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f32.floor` instruction, returning the largest integer less than or equal to `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f32::floor()`.

### `f32_trunc`

- Signature: `pub fn f32_trunc(a: f32) -> f32`
- Wasm op / alias: `f32.trunc`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f32.trunc` instruction, roundinging to the nearest integer towards zero.

This method is useful when targeting `no_std` and is equivalent to `std::f32::trunc()`.

### `f32_nearest`

- Signature: `pub fn f32_nearest(a: f32) -> f32`
- Wasm op / alias: `f32.nearest`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f32.nearest` instruction, roundinging to the nearest integer. Rounds half-way
cases to the number with an even least significant digit.

This method is useful when targeting `no_std` and is equivalent to `std::f32::round_ties_even()`.

### `f32_sqrt`

- Signature: `pub fn f32_sqrt(a: f32) -> f32`
- Wasm op / alias: `f32.sqrt`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f32.sqrt` instruction, returning the square root of the number `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f32::sqrt()`.

### `f64_ceil`

- Signature: `pub fn f64_ceil(a: f64) -> f64`
- Wasm op / alias: `f64.ceil`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f64.ceil` instruction, returning the smallest integer greater than or equal to `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f64::ceil()`.

### `f64_floor`

- Signature: `pub fn f64_floor(a: f64) -> f64`
- Wasm op / alias: `f64.floor`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f64.floor` instruction, returning the largest integer less than or equal to `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f64::floor()`.

### `f64_trunc`

- Signature: `pub fn f64_trunc(a: f64) -> f64`
- Wasm op / alias: `f64.trunc`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f64.trunc` instruction, roundinging to the nearest integer towards zero.

This method is useful when targeting `no_std` and is equivalent to `std::f64::trunc()`.

### `f64_nearest`

- Signature: `pub fn f64_nearest(a: f64) -> f64`
- Wasm op / alias: `f64.nearest`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f64.nearest` instruction, roundinging to the nearest integer. Rounds half-way
cases to the number with an even least significant digit.

This method is useful when targeting `no_std` and is equivalent to `std::f64::round_ties_even()`.

### `f64_sqrt`

- Signature: `pub fn f64_sqrt(a: f64) -> f64`
- Wasm op / alias: `f64.sqrt`
- Feature: -
- Status: unstable (`wasm_numeric_instr`, issue 133908)
- Flags / constraints: must_use

Source documentation:

Generates the `f64.sqrt` instruction, returning the square root of the number `a`.

This method is useful when targeting `no_std` and is equivalent to `std::f64::sqrt()`.

## Memory (`memory.rs`)

### `memory_size`

- Signature: `pub fn memory_size<const MEM: u32>() -> usize`
- Wasm op / alias: `memory.size`
- Feature: -
- Status: stable 1.33.0 (`simd_wasm32`)
- Flags / constraints: const `MEM`; legacy const generics; assert `MEM == 0`

Source documentation:

Corresponding intrinsic to wasm's `memory.size` instruction

This function, when called, will return the current memory size in units of
pages. The current WebAssembly page size is 65536 bytes (64 KB).

The argument `MEM` is the numerical index of which memory to return the
size of. Note that currently the WebAssembly specification only supports one
memory, so it is required that zero is passed in. The argument is present to
be forward-compatible with future WebAssembly revisions. If a nonzero
argument is passed to this function it will currently unconditionally abort.

### `memory_grow`

- Signature: `pub fn memory_grow<const MEM: u32>(delta: usize) -> usize`
- Wasm op / alias: `memory.grow`
- Feature: -
- Status: stable 1.33.0 (`simd_wasm32`)
- Flags / constraints: const `MEM`; legacy const generics; assert `MEM == 0`

Source documentation:

Corresponding intrinsic to wasm's `memory.grow` instruction

This function, when called, will attempt to grow the default linear memory
by the specified `delta` of pages. The current WebAssembly page size is
65536 bytes (64 KB). If memory is successfully grown then the previous size
of memory, in pages, is returned. If memory cannot be grown then
`usize::MAX` is returned.

The argument `MEM` is the numerical index of which memory to return the
size of. Note that currently the WebAssembly specification only supports one
memory, so it is required that zero is passed in. The argument is present to
be forward-compatible with future WebAssembly revisions. If a nonzero
argument is passed to this function it will currently unconditionally abort.

## Atomic wait/notify (`atomic.rs`)

### `memory_atomic_wait32`

- Signature: `pub unsafe fn memory_atomic_wait32(ptr: *mut i32, expression: i32, timeout_ns: i64) -> i32`
- Wasm op / alias: `memory.atomic.wait32`
- Feature: `atomics`
- Status: unstable (`stdarch_wasm_atomic_wait`, issue 77839)
- Flags / constraints: unsafe

Source documentation:

Corresponding intrinsic to wasm's `memory.atomic.wait32` instruction

This function, when called, will block the current thread if the memory
pointed to by `ptr` is equal to `expression` (performing this action
atomically).

The argument `timeout_ns` is a maximum number of nanoseconds the calling
thread will be blocked for, if it blocks. If the timeout is negative then
the calling thread will be blocked forever.

The calling thread can only be woken up with a call to the `wake` intrinsic
once it has been blocked. Changing the memory behind `ptr` will not wake
the thread once it's blocked.

#### Return value

* 0 - indicates that the thread blocked and then was woken up
* 1 - the loaded value from `ptr` didn't match `expression`, the thread
  didn't block
* 2 - the thread blocked, but the timeout expired.

### `memory_atomic_wait64`

- Signature: `pub unsafe fn memory_atomic_wait64(ptr: *mut i64, expression: i64, timeout_ns: i64) -> i32`
- Wasm op / alias: `memory.atomic.wait64`
- Feature: `atomics`
- Status: unstable (`stdarch_wasm_atomic_wait`, issue 77839)
- Flags / constraints: unsafe

Source documentation:

Corresponding intrinsic to wasm's `memory.atomic.wait64` instruction

This function, when called, will block the current thread if the memory
pointed to by `ptr` is equal to `expression` (performing this action
atomically).

The argument `timeout_ns` is a maximum number of nanoseconds the calling
thread will be blocked for, if it blocks. If the timeout is negative then
the calling thread will be blocked forever.

The calling thread can only be woken up with a call to the `wake` intrinsic
once it has been blocked. Changing the memory behind `ptr` will not wake
the thread once it's blocked.

#### Return value

* 0 - indicates that the thread blocked and then was woken up
* 1 - the loaded value from `ptr` didn't match `expression`, the thread
  didn't block
* 2 - the thread blocked, but the timeout expired.

### `memory_atomic_notify`

- Signature: `pub unsafe fn memory_atomic_notify(ptr: *mut i32, waiters: u32) -> u32`
- Wasm op / alias: `memory.atomic.notify`
- Feature: `atomics`
- Status: unstable (`stdarch_wasm_atomic_wait`, issue 77839)
- Flags / constraints: unsafe

Source documentation:

Corresponding intrinsic to wasm's `memory.atomic.notify` instruction

This function will notify a number of threads blocked on the address
indicated by `ptr`. Threads previously blocked with the `i32_atomic_wait`
and `i64_atomic_wait` functions above will be woken up.

The `waiters` argument indicates how many waiters should be woken up (a
maximum). If the value is zero no waiters are woken up.

#### Return value

Returns the number of waiters which were actually notified.

## SIMD128 (`simd128.rs`)

### `v128_load`

- Signature: `pub unsafe fn v128_load(m: *const v128) -> v128`
- Wasm op / alias: `v128.load`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Loads a `v128` vector from the given heap address.

This intrinsic will emit a load with an alignment of 1. While this is
provided for completeness it is not strictly necessary, you can also load
the pointer directly:

```rust,ignore
let a: &v128 = ...;
let value = unsafe { v128_load(a) };
// .. is the same as ..
let value = *a;
```

The alignment of the load can be configured by doing a manual load without
this intrinsic.

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 16 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i16x8_load_extend_i8x8`

- Signature: `pub unsafe fn i16x8_load_extend_i8x8(m: *const i8) -> v128`
- Wasm op / alias: `v128.load8x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load eight 8-bit integers and sign extend each one to a 16-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i16x8_load_extend_u8x8`

- Signature: `pub unsafe fn i16x8_load_extend_u8x8(m: *const u8) -> v128`
- Wasm op / alias: `v128.load8x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load eight 8-bit integers and zero extend each one to a 16-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i32x4_load_extend_i16x4`

- Signature: `pub unsafe fn i32x4_load_extend_i16x4(m: *const i16) -> v128`
- Wasm op / alias: `v128.load16x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load four 16-bit integers and sign extend each one to a 32-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i32x4_load_extend_u16x4`

- Signature: `pub unsafe fn i32x4_load_extend_u16x4(m: *const u16) -> v128`
- Wasm op / alias: `v128.load16x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load four 16-bit integers and zero extend each one to a 32-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i64x2_load_extend_i32x2`

- Signature: `pub unsafe fn i64x2_load_extend_i32x2(m: *const i32) -> v128`
- Wasm op / alias: `v128.load32x2_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load two 32-bit integers and sign extend each one to a 64-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `i64x2_load_extend_u32x2`

- Signature: `pub unsafe fn i64x2_load_extend_u32x2(m: *const u32) -> v128`
- Wasm op / alias: `v128.load32x2_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load two 32-bit integers and zero extend each one to a 64-bit lane

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load8_splat`

- Signature: `pub unsafe fn v128_load8_splat(m: *const u8) -> v128`
- Wasm op / alias: `v128.load8_splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a single element and splat to all lanes of a v128 vector.

While this intrinsic is provided for completeness it can also be replaced
with `u8x16_splat(*m)` and it should generate equivalent code (and also not
require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 1 byte from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load16_splat`

- Signature: `pub unsafe fn v128_load16_splat(m: *const u16) -> v128`
- Wasm op / alias: `v128.load16_splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a single element and splat to all lanes of a v128 vector.

While this intrinsic is provided for completeness it can also be replaced
with `u16x8_splat(*m)` and it should generate equivalent code (and also not
require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 2 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load32_splat`

- Signature: `pub unsafe fn v128_load32_splat(m: *const u32) -> v128`
- Wasm op / alias: `v128.load32_splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a single element and splat to all lanes of a v128 vector.

While this intrinsic is provided for completeness it can also be replaced
with `u32x4_splat(*m)` and it should generate equivalent code (and also not
require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 4 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load64_splat`

- Signature: `pub unsafe fn v128_load64_splat(m: *const u64) -> v128`
- Wasm op / alias: `v128.load64_splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a single element and splat to all lanes of a v128 vector.

While this intrinsic is provided for completeness it can also be replaced
with `u64x2_splat(*m)` and it should generate equivalent code (and also not
require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load32_zero`

- Signature: `pub unsafe fn v128_load32_zero(m: *const u32) -> v128`
- Wasm op / alias: `v128.load32_zero`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a 32-bit element into the low bits of the vector and sets all other
bits to zero.

This intrinsic is provided for completeness and is equivalent to `u32x4(*m,
0, 0, 0)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 4 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load64_zero`

- Signature: `pub unsafe fn v128_load64_zero(m: *const u64) -> v128`
- Wasm op / alias: `v128.load64_zero`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Load a 64-bit element into the low bits of the vector and sets all other
bits to zero.

This intrinsic is provided for completeness and is equivalent to
`u64x2_replace_lane::<0>(u64x2(0, 0), *m)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_store`

- Signature: `pub unsafe fn v128_store(m: *mut v128, a: v128)`
- Wasm op / alias: `v128.store`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe

Source documentation:

Stores a `v128` vector to the given heap address.

This intrinsic will emit a store with an alignment of 1. While this is
provided for completeness it is not strictly necessary, you can also store
the pointer directly:

```rust,ignore
let a: &mut v128 = ...;
unsafe { v128_store(a, value) };
// .. is the same as ..
*a = value;
```

The alignment of the store can be configured by doing a manual store without
this intrinsic.

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to store 16 bytes to. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned store.

### `v128_load8_lane`

- Signature: `pub unsafe fn v128_load8_lane<const L: usize>(v: v128, m: *const u8) -> v128`
- Wasm op / alias: `v128.load8_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Loads an 8-bit value from `m` and sets lane `L` of `v` to that value.

This intrinsic is provided for completeness and is equivalent to
`u8x16_replace_lane::<L>(v, *m)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 1 byte from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load16_lane`

- Signature: `pub unsafe fn v128_load16_lane<const L: usize>(v: v128, m: *const u16) -> v128`
- Wasm op / alias: `v128.load16_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Loads a 16-bit value from `m` and sets lane `L` of `v` to that value.

This intrinsic is provided for completeness and is equivalent to
`u16x8_replace_lane::<L>(v, *m)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 2 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load32_lane`

- Signature: `pub unsafe fn v128_load32_lane<const L: usize>(v: v128, m: *const u32) -> v128`
- Wasm op / alias: `v128.load32_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Loads a 32-bit value from `m` and sets lane `L` of `v` to that value.

This intrinsic is provided for completeness and is equivalent to
`u32x4_replace_lane::<L>(v, *m)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 4 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_load64_lane`

- Signature: `pub unsafe fn v128_load64_lane<const L: usize>(v: v128, m: *const u64) -> v128`
- Wasm op / alias: `v128.load64_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Loads a 64-bit value from `m` and sets lane `L` of `v` to that value.

This intrinsic is provided for completeness and is equivalent to
`u64x2_replace_lane::<L>(v, *m)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to load 8 bytes from. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned load.

### `v128_store8_lane`

- Signature: `pub unsafe fn v128_store8_lane<const L: usize>(v: v128, m: *mut u8)`
- Wasm op / alias: `v128.store8_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Stores the 8-bit value from lane `L` of `v` into `m`

This intrinsic is provided for completeness and is equivalent to
`*m = u8x16_extract_lane::<L>(v)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to store 1 byte to. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned store.

### `v128_store16_lane`

- Signature: `pub unsafe fn v128_store16_lane<const L: usize>(v: v128, m: *mut u16)`
- Wasm op / alias: `v128.store16_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Stores the 16-bit value from lane `L` of `v` into `m`

This intrinsic is provided for completeness and is equivalent to
`*m = u16x8_extract_lane::<L>(v)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to store 2 bytes to. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned store.

### `v128_store32_lane`

- Signature: `pub unsafe fn v128_store32_lane<const L: usize>(v: v128, m: *mut u32)`
- Wasm op / alias: `v128.store32_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Stores the 32-bit value from lane `L` of `v` into `m`

This intrinsic is provided for completeness and is equivalent to
`*m = u32x4_extract_lane::<L>(v)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to store 4 bytes to. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned store.

### `v128_store64_lane`

- Signature: `pub unsafe fn v128_store64_lane<const L: usize>(v: v128, m: *mut u64)`
- Wasm op / alias: `v128.store64_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: unsafe; const `L`

Source documentation:

Stores the 64-bit value from lane `L` of `v` into `m`

This intrinsic is provided for completeness and is equivalent to
`*m = u64x2_extract_lane::<L>(v)` (which doesn't require `unsafe`).

#### Unsafety

This intrinsic is unsafe because it takes a raw pointer as an argument, and
the pointer must be valid to store 8 bytes to. Note that there is no
alignment requirement on this pointer since this intrinsic performs a
1-aligned store.

### `i8x16`

- Signature: `pub const fn i8x16( a0: i8, a1: i8, a2: i8, a3: i8, a4: i8, a5: i8, a6: i8, a7: i8, a8: i8, a9: i8, a10: i8, a11: i8, a12: i8, a13: i8, a14: i8, a15: i8, ) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `u8x16`

- Signature: `pub const fn u8x16( a0: u8, a1: u8, a2: u8, a3: u8, a4: u8, a5: u8, a6: u8, a7: u8, a8: u8, a9: u8, a10: u8, a11: u8, a12: u8, a13: u8, a14: u8, a15: u8, ) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `i16x8`

- Signature: `pub const fn i16x8(a0: i16, a1: i16, a2: i16, a3: i16, a4: i16, a5: i16, a6: i16, a7: i16) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `u16x8`

- Signature: `pub const fn u16x8(a0: u16, a1: u16, a2: u16, a3: u16, a4: u16, a5: u16, a6: u16, a7: u16) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `i32x4`

- Signature: `pub const fn i32x4(a0: i32, a1: i32, a2: i32, a3: i32) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `u32x4`

- Signature: `pub const fn u32x4(a0: u32, a1: u32, a2: u32, a3: u32) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `i64x2`

- Signature: `pub const fn i64x2(a0: i64, a1: i64) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `u64x2`

- Signature: `pub const fn u64x2(a0: u64, a1: u64) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `f32x4`

- Signature: `pub const fn f32x4(a0: f32, a1: f32, a2: f32, a3: f32) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `f64x2`

- Signature: `pub const fn f64x2(a0: f64, a1: f64) -> v128`
- Wasm op / alias: `v128.const`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const fn

Source documentation:

Materializes a SIMD value from the provided operands.

If possible this will generate a `v128.const` instruction, otherwise it may
be lowered to a sequence of instructions to materialize the vector value.

### `i8x16_shuffle`

- Signature: `pub fn i8x16_shuffle< const I0: usize, const I1: usize, const I2: usize, const I3: usize, const I4: usize, const I5: usize, const I6: usize, const I7: usize, const I8: usize, const I9: usize, const I10: usize, const I11: usize, const I12: usize, const I13: usize, const I14: usize, const I15: usize, >( a: v128, b: v128, ) -> v128`
- Wasm op / alias: `i8x16.shuffle`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `I0`, `I1`, `I2`, `I3`, `I4`, `I5`, `I6`, `I7`, `I8`, `I9`, `I10`, `I11`, `I12`, `I13`, `I14`, `I15`; assert `I0..I15 < 32`

Source documentation:

Returns a new vector with lanes selected from the lanes of the two input
vectors `$a` and `$b` specified in the 16 immediate operands.

The `$a` and `$b` expressions must have type `v128`, and this function
generates a wasm instruction that is encoded with 16 bytes providing the
indices of the elements to return. The indices `i` in range [0, 15] select
the `i`-th element of `a`. The indices in range [16, 31] select the `i -
16`-th element of `b`.

Note that this is a macro due to the codegen requirements of all of the
index expressions `$i*` must be constant. A compiler error will be
generated if any of the expressions are not constant.

All indexes `$i*` must have the type `u32`.

### `i16x8_shuffle`

- Signature: `pub fn i16x8_shuffle< const I0: usize, const I1: usize, const I2: usize, const I3: usize, const I4: usize, const I5: usize, const I6: usize, const I7: usize, >( a: v128, b: v128, ) -> v128`
- Wasm op / alias: `i8x16.shuffle`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `I0`, `I1`, `I2`, `I3`, `I4`, `I5`, `I6`, `I7`; assert `I0..I7 < 16`

Source documentation:

Same as `i8x16_shuffle`, except operates as if the inputs were eight
16-bit integers, only taking 8 indices to shuffle.

Indices in the range [0, 7] select from `a` while [8, 15] select from `b`.
Note that this will generate the `i8x16.shuffle` instruction, since there
is no native `i16x8.shuffle` instruction (there is no need for one since
`i8x16.shuffle` suffices).

### `i32x4_shuffle`

- Signature: `pub fn i32x4_shuffle<const I0: usize, const I1: usize, const I2: usize, const I3: usize>( a: v128, b: v128, ) -> v128`
- Wasm op / alias: `i8x16.shuffle`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `I0`, `I1`, `I2`, `I3`; assert `I0..I3 < 8`

Source documentation:

Same as `i8x16_shuffle`, except operates as if the inputs were four
32-bit integers, only taking 4 indices to shuffle.

Indices in the range [0, 3] select from `a` while [4, 7] select from `b`.
Note that this will generate the `i8x16.shuffle` instruction, since there
is no native `i32x4.shuffle` instruction (there is no need for one since
`i8x16.shuffle` suffices).

### `i64x2_shuffle`

- Signature: `pub fn i64x2_shuffle<const I0: usize, const I1: usize>(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.shuffle`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `I0`, `I1`; assert `I0..I1 < 4`

Source documentation:

Same as `i8x16_shuffle`, except operates as if the inputs were two
64-bit integers, only taking 2 indices to shuffle.

Indices in the range [0, 1] select from `a` while [2, 3] select from `b`.
Note that this will generate the `v8x16.shuffle` instruction, since there
is no native `i64x2.shuffle` instruction (there is no need for one since
`i8x16.shuffle` suffices).

### `i8x16_extract_lane`

- Signature: `pub fn i8x16_extract_lane<const N: usize>(a: v128) -> i8`
- Wasm op / alias: `i8x16.extract_lane_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 16`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 16 packed i8 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u8x16_extract_lane`

- Signature: `pub fn u8x16_extract_lane<const N: usize>(a: v128) -> u8`
- Wasm op / alias: `i8x16.extract_lane_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 16`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 16 packed u8 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i8x16_replace_lane`

- Signature: `pub fn i8x16_replace_lane<const N: usize>(a: v128, val: i8) -> v128`
- Wasm op / alias: `i8x16.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 16`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 16 packed i8 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u8x16_replace_lane`

- Signature: `pub fn u8x16_replace_lane<const N: usize>(a: v128, val: u8) -> v128`
- Wasm op / alias: `i8x16.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 16`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 16 packed u8 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i16x8_extract_lane`

- Signature: `pub fn i16x8_extract_lane<const N: usize>(a: v128) -> i16`
- Wasm op / alias: `i16x8.extract_lane_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 8`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 8 packed i16 numbers.

Extracts a the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u16x8_extract_lane`

- Signature: `pub fn u16x8_extract_lane<const N: usize>(a: v128) -> u16`
- Wasm op / alias: `i16x8.extract_lane_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 8`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 8 packed u16 numbers.

Extracts a the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i16x8_replace_lane`

- Signature: `pub fn i16x8_replace_lane<const N: usize>(a: v128, val: i16) -> v128`
- Wasm op / alias: `i16x8.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 8`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 8 packed i16 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u16x8_replace_lane`

- Signature: `pub fn u16x8_replace_lane<const N: usize>(a: v128, val: u16) -> v128`
- Wasm op / alias: `i16x8.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 8`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 8 packed u16 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i32x4_extract_lane`

- Signature: `pub fn i32x4_extract_lane<const N: usize>(a: v128) -> i32`
- Wasm op / alias: `i32x4.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 4`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 4 packed i32 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u32x4_extract_lane`

- Signature: `pub fn u32x4_extract_lane<const N: usize>(a: v128) -> u32`
- Wasm op / alias: `i32x4.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 4 packed u32 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i32x4_replace_lane`

- Signature: `pub fn i32x4_replace_lane<const N: usize>(a: v128, val: i32) -> v128`
- Wasm op / alias: `i32x4.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 4`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 4 packed i32 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u32x4_replace_lane`

- Signature: `pub fn u32x4_replace_lane<const N: usize>(a: v128, val: u32) -> v128`
- Wasm op / alias: `i32x4.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 4 packed u32 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i64x2_extract_lane`

- Signature: `pub fn i64x2_extract_lane<const N: usize>(a: v128) -> i64`
- Wasm op / alias: `i64x2.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 2`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 2 packed i64 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u64x2_extract_lane`

- Signature: `pub fn u64x2_extract_lane<const N: usize>(a: v128) -> u64`
- Wasm op / alias: `i64x2.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 2 packed u64 numbers.

Extracts the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i64x2_replace_lane`

- Signature: `pub fn i64x2_replace_lane<const N: usize>(a: v128, val: i64) -> v128`
- Wasm op / alias: `i64x2.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 2`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 2 packed i64 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `u64x2_replace_lane`

- Signature: `pub fn u64x2_replace_lane<const N: usize>(a: v128, val: u64) -> v128`
- Wasm op / alias: `i64x2.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 2 packed u64 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `f32x4_extract_lane`

- Signature: `pub fn f32x4_extract_lane<const N: usize>(a: v128) -> f32`
- Wasm op / alias: `f32x4.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 4`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 4 packed f32 numbers.

Extracts the scalar value of lane specified fn the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `f32x4_replace_lane`

- Signature: `pub fn f32x4_replace_lane<const N: usize>(a: v128, val: f32) -> v128`
- Wasm op / alias: `f32x4.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 4`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 4 packed f32 numbers.

Replaces the scalar value of lane specified fn the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `f64x2_extract_lane`

- Signature: `pub fn f64x2_extract_lane<const N: usize>(a: v128) -> f64`
- Wasm op / alias: `f64x2.extract_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 2`

Source documentation:

Extracts a lane from a 128-bit vector interpreted as 2 packed f64 numbers.

Extracts the scalar value of lane specified fn the immediate mode operand
`N` from `a`. If `N` fs out of bounds then it is a compile time error.

### `f64x2_replace_lane`

- Signature: `pub fn f64x2_replace_lane<const N: usize>(a: v128, val: f64) -> v128`
- Wasm op / alias: `f64x2.replace_lane`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: const `N`; assert `N < 2`

Source documentation:

Replaces a lane from a 128-bit vector interpreted as 2 packed f64 numbers.

Replaces the scalar value of lane specified in the immediate mode operand
`N` from `a`. If `N` is out of bounds then it is a compile time error.

### `i8x16_swizzle`

- Signature: `pub fn i8x16_swizzle(a: v128, s: v128) -> v128`
- Wasm op / alias: `i8x16.swizzle`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns a new vector with lanes selected from the lanes of the first input
vector `a` specified in the second input vector `s`.

The indices `i` in range [0, 15] select the `i`-th element of `a`. For
indices outside of the range the resulting lane is 0.

### `i8x16_splat`

- Signature: `pub fn i8x16_splat(a: i8) -> v128`
- Wasm op / alias: `i8x16.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 16 lanes.

### `u8x16_splat`

- Signature: `pub fn u8x16_splat(a: u8) -> v128`
- Wasm op / alias: `i8x16.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 16 lanes.

### `i16x8_splat`

- Signature: `pub fn i16x8_splat(a: i16) -> v128`
- Wasm op / alias: `i16x8.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Construct a vector with `x` replicated to all 8 lanes.

### `u16x8_splat`

- Signature: `pub fn u16x8_splat(a: u16) -> v128`
- Wasm op / alias: `i16x8.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Construct a vector with `x` replicated to all 8 lanes.

### `i32x4_splat`

- Signature: `pub fn i32x4_splat(a: i32) -> v128`
- Wasm op / alias: `i32x4.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 4 lanes.

### `u32x4_splat`

- Signature: `pub fn u32x4_splat(a: u32) -> v128`
- Wasm op / alias: `i32x4.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 4 lanes.

### `i64x2_splat`

- Signature: `pub fn i64x2_splat(a: i64) -> v128`
- Wasm op / alias: `i64x2.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Construct a vector with `x` replicated to all 2 lanes.

### `u64x2_splat`

- Signature: `pub fn u64x2_splat(a: u64) -> v128`
- Wasm op / alias: `u64x2.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Construct a vector with `x` replicated to all 2 lanes.

### `f32x4_splat`

- Signature: `pub fn f32x4_splat(a: f32) -> v128`
- Wasm op / alias: `f32x4.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 4 lanes.

### `f64x2_splat`

- Signature: `pub fn f64x2_splat(a: f64) -> v128`
- Wasm op / alias: `f64x2.splat`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Creates a vector with identical lanes.

Constructs a vector with `x` replicated to all 2 lanes.

### `i8x16_eq`

- Signature: `pub fn i8x16_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `i8x16_ne`

- Signature: `pub fn i8x16_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `i8x16_lt`

- Signature: `pub fn i8x16_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.lt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u8x16_lt`

- Signature: `pub fn u8x16_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.lt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i8x16_gt`

- Signature: `pub fn i8x16_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.gt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u8x16_gt`

- Signature: `pub fn u8x16_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.gt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i8x16_le`

- Signature: `pub fn i8x16_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.le_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u8x16_le`

- Signature: `pub fn u8x16_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.le_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i8x16_ge`

- Signature: `pub fn i8x16_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.ge_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u8x16_ge`

- Signature: `pub fn u8x16_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.ge_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 16 eight-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i16x8_eq`

- Signature: `pub fn i16x8_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `i16x8_ne`

- Signature: `pub fn i16x8_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `i16x8_lt`

- Signature: `pub fn i16x8_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.lt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u16x8_lt`

- Signature: `pub fn u16x8_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.lt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i16x8_gt`

- Signature: `pub fn i16x8_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.gt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u16x8_gt`

- Signature: `pub fn u16x8_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.gt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i16x8_le`

- Signature: `pub fn i16x8_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.le_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u16x8_le`

- Signature: `pub fn u16x8_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.le_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i16x8_ge`

- Signature: `pub fn i16x8_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.ge_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u16x8_ge`

- Signature: `pub fn u16x8_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.ge_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 8 sixteen-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i32x4_eq`

- Signature: `pub fn i32x4_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `i32x4_ne`

- Signature: `pub fn i32x4_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `i32x4_lt`

- Signature: `pub fn i32x4_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.lt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u32x4_lt`

- Signature: `pub fn u32x4_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.lt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i32x4_gt`

- Signature: `pub fn i32x4_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.gt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u32x4_gt`

- Signature: `pub fn u32x4_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.gt_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i32x4_le`

- Signature: `pub fn i32x4_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.le_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `u32x4_le`

- Signature: `pub fn u32x4_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.le_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i32x4_ge`

- Signature: `pub fn i32x4_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.ge_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `u32x4_ge`

- Signature: `pub fn u32x4_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.ge_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
unsigned integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i64x2_eq`

- Signature: `pub fn i64x2_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `i64x2_ne`

- Signature: `pub fn i64x2_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
integers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `i64x2_lt`

- Signature: `pub fn i64x2_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.lt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i64x2_gt`

- Signature: `pub fn i64x2_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.gt_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `i64x2_le`

- Signature: `pub fn i64x2_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.le_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `i64x2_ge`

- Signature: `pub fn i64x2_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.ge_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
signed integers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `f32x4_eq`

- Signature: `pub fn f32x4_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `f32x4_ne`

- Signature: `pub fn f32x4_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `f32x4_lt`

- Signature: `pub fn f32x4_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.lt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `f32x4_gt`

- Signature: `pub fn f32x4_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.gt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `f32x4_le`

- Signature: `pub fn f32x4_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.le`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `f32x4_ge`

- Signature: `pub fn f32x4_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.ge`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 4 thirty-two-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `f64x2_eq`

- Signature: `pub fn f64x2_eq(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.eq`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the corresponding input elements
were equal, or all zeros otherwise.

### `f64x2_ne`

- Signature: `pub fn f64x2_ne(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.ne`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the corresponding input elements
were not equal, or all zeros otherwise.

### `f64x2_lt`

- Signature: `pub fn f64x2_lt(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.lt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `f64x2_gt`

- Signature: `pub fn f64x2_gt(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.gt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `f64x2_le`

- Signature: `pub fn f64x2_le(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.le`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is less than the right element, or all zeros otherwise.

### `f64x2_ge`

- Signature: `pub fn f64x2_ge(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.ge`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares two 128-bit vectors as if they were two vectors of 2 sixty-four-bit
floating point numbers.

Returns a new vector where each lane is all ones if the lane-wise left
element is greater than the right element, or all zeros otherwise.

### `v128_not`

- Signature: `pub fn v128_not(a: v128) -> v128`
- Wasm op / alias: `v128.not`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Flips each bit of the 128-bit input vector.

### `v128_and`

- Signature: `pub fn v128_and(a: v128, b: v128) -> v128`
- Wasm op / alias: `v128.and`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Performs a bitwise and of the two input 128-bit vectors, returning the
resulting vector.

### `v128_andnot`

- Signature: `pub fn v128_andnot(a: v128, b: v128) -> v128`
- Wasm op / alias: `v128.andnot`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Bitwise AND of bits of `a` and the logical inverse of bits of `b`.

This operation is equivalent to `v128.and(a, v128.not(b))`

### `v128_or`

- Signature: `pub fn v128_or(a: v128, b: v128) -> v128`
- Wasm op / alias: `v128.or`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Performs a bitwise or of the two input 128-bit vectors, returning the
resulting vector.

### `v128_xor`

- Signature: `pub fn v128_xor(a: v128, b: v128) -> v128`
- Wasm op / alias: `v128.xor`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Performs a bitwise xor of the two input 128-bit vectors, returning the
resulting vector.

### `v128_bitselect`

- Signature: `pub fn v128_bitselect(v1: v128, v2: v128, c: v128) -> v128`
- Wasm op / alias: `v128.bitselect`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Use the bitmask in `c` to select bits from `v1` when 1 and `v2` when 0.

### `v128_any_true`

- Signature: `pub fn v128_any_true(a: v128) -> bool`
- Wasm op / alias: `v128.any_true`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns `true` if any bit in `a` is set, or `false` otherwise.

### `i8x16_abs`

- Signature: `pub fn i8x16_abs(a: v128) -> v128`
- Wasm op / alias: `i8x16.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise wrapping absolute value.

### `i8x16_neg`

- Signature: `pub fn i8x16_neg(a: v128) -> v128`
- Wasm op / alias: `i8x16.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates a 128-bit vectors interpreted as sixteen 8-bit signed integers

### `i8x16_popcnt`

- Signature: `pub fn i8x16_popcnt(v: v128) -> v128`
- Wasm op / alias: `i8x16.popcnt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Count the number of bits set to one within each lane.

### `i8x16_all_true`

- Signature: `pub fn i8x16_all_true(a: v128) -> bool`
- Wasm op / alias: `i8x16.all_true`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns true if all lanes are non-zero, false otherwise.

### `i8x16_bitmask`

- Signature: `pub fn i8x16_bitmask(a: v128) -> u16`
- Wasm op / alias: `i8x16.bitmask`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Extracts the high bit for each lane in `a` and produce a scalar mask with
all bits concatenated.

### `i8x16_narrow_i16x8`

- Signature: `pub fn i8x16_narrow_i16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.narrow_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts two input vectors into a smaller lane vector by narrowing each
lane.

Signed saturation to 0x7f or 0x80 is used and the input lanes are always
interpreted as signed integers.

### `u8x16_narrow_i16x8`

- Signature: `pub fn u8x16_narrow_i16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.narrow_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts two input vectors into a smaller lane vector by narrowing each
lane.

Signed saturation to 0x00 or 0xff is used and the input lanes are always
interpreted as signed integers.

### `i8x16_shl`

- Signature: `pub fn i8x16_shl(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i8x16.shl`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the left by the specified number of bits.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i8x16_shr`

- Signature: `pub fn i8x16_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i8x16.shr_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, sign
extending.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `u8x16_shr`

- Signature: `pub fn u8x16_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i8x16.shr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, shifting in
zeros.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i8x16_add`

- Signature: `pub fn i8x16_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed sixteen 8-bit integers.

### `i8x16_add_sat`

- Signature: `pub fn i8x16_add_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.add_sat_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed sixteen 8-bit signed
integers, saturating on overflow to `i8::MAX`.

### `u8x16_add_sat`

- Signature: `pub fn u8x16_add_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.add_sat_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed sixteen 8-bit unsigned
integers, saturating on overflow to `u8::MAX`.

### `i8x16_sub`

- Signature: `pub fn i8x16_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed sixteen 8-bit integers.

### `i8x16_sub_sat`

- Signature: `pub fn i8x16_sub_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.sub_sat_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed sixteen 8-bit
signed integers, saturating on overflow to `i8::MIN`.

### `u8x16_sub_sat`

- Signature: `pub fn u8x16_sub_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.sub_sat_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed sixteen 8-bit
unsigned integers, saturating on overflow to 0.

### `i8x16_min`

- Signature: `pub fn i8x16_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.min_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the minimum of
each pair.

### `u8x16_min`

- Signature: `pub fn u8x16_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.min_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the minimum of
each pair.

### `i8x16_max`

- Signature: `pub fn i8x16_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.max_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the maximum of
each pair.

### `u8x16_max`

- Signature: `pub fn u8x16_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.max_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the maximum of
each pair.

### `u8x16_avgr`

- Signature: `pub fn u8x16_avgr(a: v128, b: v128) -> v128`
- Wasm op / alias: `i8x16.avgr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding average.

### `i16x8_extadd_pairwise_i8x16`

- Signature: `pub fn i16x8_extadd_pairwise_i8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extadd_pairwise_i8x16_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Integer extended pairwise addition producing extended results
(twice wider results than the inputs).

### `i16x8_extadd_pairwise_u8x16`

- Signature: `pub fn i16x8_extadd_pairwise_u8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extadd_pairwise_i8x16_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Integer extended pairwise addition producing extended results
(twice wider results than the inputs).

### `i16x8_abs`

- Signature: `pub fn i16x8_abs(a: v128) -> v128`
- Wasm op / alias: `i16x8.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise wrapping absolute value.

### `i16x8_neg`

- Signature: `pub fn i16x8_neg(a: v128) -> v128`
- Wasm op / alias: `i16x8.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates a 128-bit vectors interpreted as eight 16-bit signed integers

### `i16x8_q15mulr_sat`

- Signature: `pub fn i16x8_q15mulr_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.q15mulr_sat_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise saturating rounding multiplication in Q15 format.

### `i16x8_all_true`

- Signature: `pub fn i16x8_all_true(a: v128) -> bool`
- Wasm op / alias: `i16x8.all_true`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns true if all lanes are non-zero, false otherwise.

### `i16x8_bitmask`

- Signature: `pub fn i16x8_bitmask(a: v128) -> u8`
- Wasm op / alias: `i16x8.bitmask`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Extracts the high bit for each lane in `a` and produce a scalar mask with
all bits concatenated.

### `i16x8_narrow_i32x4`

- Signature: `pub fn i16x8_narrow_i32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.narrow_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts two input vectors into a smaller lane vector by narrowing each
lane.

Signed saturation to 0x7fff or 0x8000 is used and the input lanes are always
interpreted as signed integers.

### `u16x8_narrow_i32x4`

- Signature: `pub fn u16x8_narrow_i32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.narrow_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts two input vectors into a smaller lane vector by narrowing each
lane.

Signed saturation to 0x0000 or 0xffff is used and the input lanes are always
interpreted as signed integers.

### `i16x8_extend_low_i8x16`

- Signature: `pub fn i16x8_extend_low_i8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extend_low_i8x16_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, sign extended.

### `i16x8_extend_high_i8x16`

- Signature: `pub fn i16x8_extend_high_i8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extend_high_i8x16_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, sign extended.

### `i16x8_extend_low_u8x16`

- Signature: `pub fn i16x8_extend_low_u8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extend_low_i8x16_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, zero extended.

### `i16x8_extend_high_u8x16`

- Signature: `pub fn i16x8_extend_high_u8x16(a: v128) -> v128`
- Wasm op / alias: `i16x8.extend_high_i8x16_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, zero extended.

### `i16x8_shl`

- Signature: `pub fn i16x8_shl(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i16x8.shl`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the left by the specified number of bits.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i16x8_shr`

- Signature: `pub fn i16x8_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i16x8.shr_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, sign
extending.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `u16x8_shr`

- Signature: `pub fn u16x8_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i16x8.shr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, shifting in
zeros.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i16x8_add`

- Signature: `pub fn i16x8_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed eight 16-bit integers.

### `i16x8_add_sat`

- Signature: `pub fn i16x8_add_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.add_sat_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed eight 16-bit signed
integers, saturating on overflow to `i16::MAX`.

### `u16x8_add_sat`

- Signature: `pub fn u16x8_add_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.add_sat_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed eight 16-bit unsigned
integers, saturating on overflow to `u16::MAX`.

### `i16x8_sub`

- Signature: `pub fn i16x8_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed eight 16-bit integers.

### `i16x8_sub_sat`

- Signature: `pub fn i16x8_sub_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.sub_sat_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed eight 16-bit
signed integers, saturating on overflow to `i16::MIN`.

### `u16x8_sub_sat`

- Signature: `pub fn u16x8_sub_sat(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.sub_sat_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed eight 16-bit
unsigned integers, saturating on overflow to 0.

### `i16x8_mul`

- Signature: `pub fn i16x8_mul(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.mul`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Multiplies two 128-bit vectors as if they were two packed eight 16-bit
signed integers.

### `i16x8_min`

- Signature: `pub fn i16x8_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.min_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the minimum of
each pair.

### `u16x8_min`

- Signature: `pub fn u16x8_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.min_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the minimum of
each pair.

### `i16x8_max`

- Signature: `pub fn i16x8_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.max_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the maximum of
each pair.

### `u16x8_max`

- Signature: `pub fn u16x8_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.max_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the maximum of
each pair.

### `u16x8_avgr`

- Signature: `pub fn u16x8_avgr(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.avgr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding average.

### `i16x8_extmul_low_i8x16`

- Signature: `pub fn i16x8_extmul_low_i8x16(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.extmul_low_i8x16_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i16x8_mul(i16x8_extend_low_i8x16(a), i16x8_extend_low_i8x16(b))`

### `i16x8_extmul_high_i8x16`

- Signature: `pub fn i16x8_extmul_high_i8x16(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.extmul_high_i8x16_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i16x8_mul(i16x8_extend_high_i8x16(a), i16x8_extend_high_i8x16(b))`

### `i16x8_extmul_low_u8x16`

- Signature: `pub fn i16x8_extmul_low_u8x16(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.extmul_low_i8x16_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i16x8_mul(i16x8_extend_low_u8x16(a), i16x8_extend_low_u8x16(b))`

### `i16x8_extmul_high_u8x16`

- Signature: `pub fn i16x8_extmul_high_u8x16(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.extmul_high_i8x16_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i16x8_mul(i16x8_extend_high_u8x16(a), i16x8_extend_high_u8x16(b))`

### `i32x4_extadd_pairwise_i16x8`

- Signature: `pub fn i32x4_extadd_pairwise_i16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extadd_pairwise_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Integer extended pairwise addition producing extended results
(twice wider results than the inputs).

### `i32x4_extadd_pairwise_u16x8`

- Signature: `pub fn i32x4_extadd_pairwise_u16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extadd_pairwise_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Integer extended pairwise addition producing extended results
(twice wider results than the inputs).

### `i32x4_abs`

- Signature: `pub fn i32x4_abs(a: v128) -> v128`
- Wasm op / alias: `i32x4.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise wrapping absolute value.

### `i32x4_neg`

- Signature: `pub fn i32x4_neg(a: v128) -> v128`
- Wasm op / alias: `i32x4.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates a 128-bit vectors interpreted as four 32-bit signed integers

### `i32x4_all_true`

- Signature: `pub fn i32x4_all_true(a: v128) -> bool`
- Wasm op / alias: `i32x4.all_true`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns true if all lanes are non-zero, false otherwise.

### `i32x4_bitmask`

- Signature: `pub fn i32x4_bitmask(a: v128) -> u8`
- Wasm op / alias: `i32x4.bitmask`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Extracts the high bit for each lane in `a` and produce a scalar mask with
all bits concatenated.

### `i32x4_extend_low_i16x8`

- Signature: `pub fn i32x4_extend_low_i16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extend_low_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, sign extended.

### `i32x4_extend_high_i16x8`

- Signature: `pub fn i32x4_extend_high_i16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extend_high_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, sign extended.

### `i32x4_extend_low_u16x8`

- Signature: `pub fn i32x4_extend_low_u16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extend_low_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, zero extended.

### `i32x4_extend_high_u16x8`

- Signature: `pub fn i32x4_extend_high_u16x8(a: v128) -> v128`
- Wasm op / alias: `i32x4.extend_high_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, zero extended.

### `i32x4_shl`

- Signature: `pub fn i32x4_shl(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i32x4.shl`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the left by the specified number of bits.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i32x4_shr`

- Signature: `pub fn i32x4_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i32x4.shr_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, sign
extending.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `u32x4_shr`

- Signature: `pub fn u32x4_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i32x4.shr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, shifting in
zeros.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i32x4_add`

- Signature: `pub fn i32x4_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed four 32-bit integers.

### `i32x4_sub`

- Signature: `pub fn i32x4_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed four 32-bit integers.

### `i32x4_mul`

- Signature: `pub fn i32x4_mul(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.mul`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Multiplies two 128-bit vectors as if they were two packed four 32-bit
signed integers.

### `i32x4_min`

- Signature: `pub fn i32x4_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.min_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the minimum of
each pair.

### `u32x4_min`

- Signature: `pub fn u32x4_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.min_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the minimum of
each pair.

### `i32x4_max`

- Signature: `pub fn i32x4_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.max_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise signed integers, and returns the maximum of
each pair.

### `u32x4_max`

- Signature: `pub fn u32x4_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.max_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Compares lane-wise unsigned integers, and returns the maximum of
each pair.

### `i32x4_dot_i16x8`

- Signature: `pub fn i32x4_dot_i16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.dot_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise multiply signed 16-bit integers in the two input vectors and add
adjacent pairs of the full 32-bit results.

### `i32x4_extmul_low_i16x8`

- Signature: `pub fn i32x4_extmul_low_i16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.extmul_low_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i32x4_mul(i32x4_extend_low_i16x8_s(a), i32x4_extend_low_i16x8_s(b))`

### `i32x4_extmul_high_i16x8`

- Signature: `pub fn i32x4_extmul_high_i16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.extmul_high_i16x8_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i32x4_mul(i32x4_extend_high_i16x8_s(a), i32x4_extend_high_i16x8_s(b))`

### `i32x4_extmul_low_u16x8`

- Signature: `pub fn i32x4_extmul_low_u16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.extmul_low_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i32x4_mul(i32x4_extend_low_u16x8(a), i32x4_extend_low_u16x8(b))`

### `i32x4_extmul_high_u16x8`

- Signature: `pub fn i32x4_extmul_high_u16x8(a: v128, b: v128) -> v128`
- Wasm op / alias: `i32x4.extmul_high_i16x8_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i32x4_mul(i32x4_extend_high_u16x8(a), i32x4_extend_high_u16x8(b))`

### `i64x2_abs`

- Signature: `pub fn i64x2_abs(a: v128) -> v128`
- Wasm op / alias: `i64x2.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise wrapping absolute value.

### `i64x2_neg`

- Signature: `pub fn i64x2_neg(a: v128) -> v128`
- Wasm op / alias: `i64x2.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates a 128-bit vectors interpreted as two 64-bit signed integers

### `i64x2_all_true`

- Signature: `pub fn i64x2_all_true(a: v128) -> bool`
- Wasm op / alias: `i64x2.all_true`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Returns true if all lanes are non-zero, false otherwise.

### `i64x2_bitmask`

- Signature: `pub fn i64x2_bitmask(a: v128) -> u8`
- Wasm op / alias: `i64x2.bitmask`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Extracts the high bit for each lane in `a` and produce a scalar mask with
all bits concatenated.

### `i64x2_extend_low_i32x4`

- Signature: `pub fn i64x2_extend_low_i32x4(a: v128) -> v128`
- Wasm op / alias: `i64x2.extend_low_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, sign extended.

### `i64x2_extend_high_i32x4`

- Signature: `pub fn i64x2_extend_high_i32x4(a: v128) -> v128`
- Wasm op / alias: `i64x2.extend_high_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, sign extended.

### `i64x2_extend_low_u32x4`

- Signature: `pub fn i64x2_extend_low_u32x4(a: v128) -> v128`
- Wasm op / alias: `i64x2.extend_low_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts low half of the smaller lane vector to a larger lane
vector, zero extended.

### `i64x2_extend_high_u32x4`

- Signature: `pub fn i64x2_extend_high_u32x4(a: v128) -> v128`
- Wasm op / alias: `i64x2.extend_high_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts high half of the smaller lane vector to a larger lane
vector, zero extended.

### `i64x2_shl`

- Signature: `pub fn i64x2_shl(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i64x2.shl`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the left by the specified number of bits.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i64x2_shr`

- Signature: `pub fn i64x2_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i64x2.shr_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, sign
extending.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `u64x2_shr`

- Signature: `pub fn u64x2_shr(a: v128, amt: u32) -> v128`
- Wasm op / alias: `i64x2.shr_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Shifts each lane to the right by the specified number of bits, shifting in
zeros.

Only the low bits of the shift amount are used if the shift amount is
greater than the lane width.

### `i64x2_add`

- Signature: `pub fn i64x2_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Adds two 128-bit vectors as if they were two packed two 64-bit integers.

### `i64x2_sub`

- Signature: `pub fn i64x2_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Subtracts two 128-bit vectors as if they were two packed two 64-bit integers.

### `i64x2_mul`

- Signature: `pub fn i64x2_mul(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.mul`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Multiplies two 128-bit vectors as if they were two packed two 64-bit integers.

### `i64x2_extmul_low_i32x4`

- Signature: `pub fn i64x2_extmul_low_i32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.extmul_low_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i64x2_mul(i64x2_extend_low_i32x4_s(a), i64x2_extend_low_i32x4_s(b))`

### `i64x2_extmul_high_i32x4`

- Signature: `pub fn i64x2_extmul_high_i32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.extmul_high_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i64x2_mul(i64x2_extend_high_i32x4_s(a), i64x2_extend_high_i32x4_s(b))`

### `i64x2_extmul_low_u32x4`

- Signature: `pub fn i64x2_extmul_low_u32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.extmul_low_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i64x2_mul(i64x2_extend_low_i32x4_u(a), i64x2_extend_low_i32x4_u(b))`

### `i64x2_extmul_high_u32x4`

- Signature: `pub fn i64x2_extmul_high_u32x4(a: v128, b: v128) -> v128`
- Wasm op / alias: `i64x2.extmul_high_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise integer extended multiplication producing twice wider result than
the inputs.

Equivalent of `i64x2_mul(i64x2_extend_high_i32x4_u(a), i64x2_extend_high_i32x4_u(b))`

### `f32x4_ceil`

- Signature: `pub fn f32x4_ceil(a: v128) -> v128`
- Wasm op / alias: `f32x4.ceil`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value not smaller than the input.

### `f32x4_floor`

- Signature: `pub fn f32x4_floor(a: v128) -> v128`
- Wasm op / alias: `f32x4.floor`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value not greater than the input.

### `f32x4_trunc`

- Signature: `pub fn f32x4_trunc(a: v128) -> v128`
- Wasm op / alias: `f32x4.trunc`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value with the magnitude not
larger than the input.

### `f32x4_nearest`

- Signature: `pub fn f32x4_nearest(a: v128) -> v128`
- Wasm op / alias: `f32x4.nearest`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value; if two values are equally
near, rounds to the even one.

### `f32x4_abs`

- Signature: `pub fn f32x4_abs(a: v128) -> v128`
- Wasm op / alias: `f32x4.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the absolute value of each lane of a 128-bit vector interpreted
as four 32-bit floating point numbers.

### `f32x4_neg`

- Signature: `pub fn f32x4_neg(a: v128) -> v128`
- Wasm op / alias: `f32x4.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates each lane of a 128-bit vector interpreted as four 32-bit floating
point numbers.

### `f32x4_sqrt`

- Signature: `pub fn f32x4_sqrt(a: v128) -> v128`
- Wasm op / alias: `f32x4.sqrt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the square root of each lane of a 128-bit vector interpreted as
four 32-bit floating point numbers.

### `f32x4_add`

- Signature: `pub fn f32x4_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise addition of two 128-bit vectors interpreted as four 32-bit
floating point numbers.

### `f32x4_sub`

- Signature: `pub fn f32x4_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise subtraction of two 128-bit vectors interpreted as four 32-bit
floating point numbers.

### `f32x4_mul`

- Signature: `pub fn f32x4_mul(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.mul`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise multiplication of two 128-bit vectors interpreted as four 32-bit
floating point numbers.

### `f32x4_div`

- Signature: `pub fn f32x4_div(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.div`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise division of two 128-bit vectors interpreted as four 32-bit
floating point numbers.

### `f32x4_min`

- Signature: `pub fn f32x4_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.min`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the lane-wise minimum of two 128-bit vectors interpreted
as four 32-bit floating point numbers.

### `f32x4_max`

- Signature: `pub fn f32x4_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.max`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the lane-wise minimum of two 128-bit vectors interpreted
as four 32-bit floating point numbers.

### `f32x4_pmin`

- Signature: `pub fn f32x4_pmin(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.pmin`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise minimum value, defined as `b < a ? b : a`

### `f32x4_pmax`

- Signature: `pub fn f32x4_pmax(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.pmax`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise maximum value, defined as `a < b ? b : a`

### `f64x2_ceil`

- Signature: `pub fn f64x2_ceil(a: v128) -> v128`
- Wasm op / alias: `f64x2.ceil`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value not smaller than the input.

### `f64x2_floor`

- Signature: `pub fn f64x2_floor(a: v128) -> v128`
- Wasm op / alias: `f64x2.floor`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value not greater than the input.

### `f64x2_trunc`

- Signature: `pub fn f64x2_trunc(a: v128) -> v128`
- Wasm op / alias: `f64x2.trunc`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value with the magnitude not
larger than the input.

### `f64x2_nearest`

- Signature: `pub fn f64x2_nearest(a: v128) -> v128`
- Wasm op / alias: `f64x2.nearest`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise rounding to the nearest integral value; if two values are equally
near, rounds to the even one.

### `f64x2_abs`

- Signature: `pub fn f64x2_abs(a: v128) -> v128`
- Wasm op / alias: `f64x2.abs`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the absolute value of each lane of a 128-bit vector interpreted
as two 64-bit floating point numbers.

### `f64x2_neg`

- Signature: `pub fn f64x2_neg(a: v128) -> v128`
- Wasm op / alias: `f64x2.neg`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Negates each lane of a 128-bit vector interpreted as two 64-bit floating
point numbers.

### `f64x2_sqrt`

- Signature: `pub fn f64x2_sqrt(a: v128) -> v128`
- Wasm op / alias: `f64x2.sqrt`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the square root of each lane of a 128-bit vector interpreted as
two 64-bit floating point numbers.

### `f64x2_add`

- Signature: `pub fn f64x2_add(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.add`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise add of two 128-bit vectors interpreted as two 64-bit
floating point numbers.

### `f64x2_sub`

- Signature: `pub fn f64x2_sub(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.sub`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise subtract of two 128-bit vectors interpreted as two 64-bit
floating point numbers.

### `f64x2_mul`

- Signature: `pub fn f64x2_mul(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.mul`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise multiply of two 128-bit vectors interpreted as two 64-bit
floating point numbers.

### `f64x2_div`

- Signature: `pub fn f64x2_div(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.div`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise divide of two 128-bit vectors interpreted as two 64-bit
floating point numbers.

### `f64x2_min`

- Signature: `pub fn f64x2_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.min`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the lane-wise minimum of two 128-bit vectors interpreted
as two 64-bit floating point numbers.

### `f64x2_max`

- Signature: `pub fn f64x2_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.max`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Calculates the lane-wise maximum of two 128-bit vectors interpreted
as two 64-bit floating point numbers.

### `f64x2_pmin`

- Signature: `pub fn f64x2_pmin(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.pmin`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise minimum value, defined as `b < a ? b : a`

### `f64x2_pmax`

- Signature: `pub fn f64x2_pmax(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.pmax`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise maximum value, defined as `a < b ? b : a`

### `i32x4_trunc_sat_f32x4`

- Signature: `pub fn i32x4_trunc_sat_f32x4(a: v128) -> v128`
- Wasm op / alias: `i32x4.trunc_sat_f32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts a 128-bit vector interpreted as four 32-bit floating point numbers
into a 128-bit vector of four 32-bit signed integers.

NaN is converted to 0 and if it's out of bounds it becomes the nearest
representable intger.

### `u32x4_trunc_sat_f32x4`

- Signature: `pub fn u32x4_trunc_sat_f32x4(a: v128) -> v128`
- Wasm op / alias: `i32x4.trunc_sat_f32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts a 128-bit vector interpreted as four 32-bit floating point numbers
into a 128-bit vector of four 32-bit unsigned integers.

NaN is converted to 0 and if it's out of bounds it becomes the nearest
representable intger.

### `f32x4_convert_i32x4`

- Signature: `pub fn f32x4_convert_i32x4(a: v128) -> v128`
- Wasm op / alias: `f32x4.convert_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts a 128-bit vector interpreted as four 32-bit signed integers into a
128-bit vector of four 32-bit floating point numbers.

### `f32x4_convert_u32x4`

- Signature: `pub fn f32x4_convert_u32x4(a: v128) -> v128`
- Wasm op / alias: `f32x4.convert_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Converts a 128-bit vector interpreted as four 32-bit unsigned integers into a
128-bit vector of four 32-bit floating point numbers.

### `i32x4_trunc_sat_f64x2_zero`

- Signature: `pub fn i32x4_trunc_sat_f64x2_zero(a: v128) -> v128`
- Wasm op / alias: `i32x4.trunc_sat_f64x2_s_zero`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Saturating conversion of the two double-precision floating point lanes to
two lower integer lanes using the IEEE `convertToIntegerTowardZero`
function.

The two higher lanes of the result are initialized to zero. If any input
lane is a NaN, the resulting lane is 0. If the rounded integer value of a
lane is outside the range of the destination type, the result is saturated
to the nearest representable integer value.

### `u32x4_trunc_sat_f64x2_zero`

- Signature: `pub fn u32x4_trunc_sat_f64x2_zero(a: v128) -> v128`
- Wasm op / alias: `i32x4.trunc_sat_f64x2_u_zero`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Saturating conversion of the two double-precision floating point lanes to
two lower integer lanes using the IEEE `convertToIntegerTowardZero`
function.

The two higher lanes of the result are initialized to zero. If any input
lane is a NaN, the resulting lane is 0. If the rounded integer value of a
lane is outside the range of the destination type, the result is saturated
to the nearest representable integer value.

### `f64x2_convert_low_i32x4`

- Signature: `pub fn f64x2_convert_low_i32x4(a: v128) -> v128`
- Wasm op / alias: `f64x2.convert_low_i32x4_s`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise conversion from integer to floating point.

### `f64x2_convert_low_u32x4`

- Signature: `pub fn f64x2_convert_low_u32x4(a: v128) -> v128`
- Wasm op / alias: `f64x2.convert_low_i32x4_u`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Lane-wise conversion from integer to floating point.

### `f32x4_demote_f64x2_zero`

- Signature: `pub fn f32x4_demote_f64x2_zero(a: v128) -> v128`
- Wasm op / alias: `f32x4.demote_f64x2_zero`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Conversion of the two double-precision floating point lanes to two lower
single-precision lanes of the result. The two higher lanes of the result are
initialized to zero. If the conversion result is not representable as a
single-precision floating point number, it is rounded to the nearest-even
representable number.

### `f64x2_promote_low_f32x4`

- Signature: `pub fn f64x2_promote_low_f32x4(a: v128) -> v128`
- Wasm op / alias: `f64x2.promote_low_f32x4`, `f32x4.promote_low_f32x4`
- Feature: `simd128`
- Status: stable 1.54.0 (`wasm_simd`)
- Flags / constraints: -

Source documentation:

Conversion of the two lower single-precision floating point lanes to the two
double-precision lanes of the result.

## Relaxed SIMD (`relaxed_simd.rs`)

### `i8x16_relaxed_swizzle`

- Signature: `pub fn i8x16_relaxed_swizzle(a: v128, s: v128) -> v128`
- Wasm op / alias: `i8x16.relaxed_swizzle`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `i8x16_swizzle(a, s)` which selects lanes from `a`
using indices in `s`.

Indices in the range `[0,15]` will select the `i`-th element of `a`.
If the high bit of any element of `s` is set (meaning 128 or greater) then
the corresponding output lane is guaranteed to be zero. Otherwise if the
element of `s` is within the range `[16,128)` then the output lane is either
0 or `a[s[i] % 16]` depending on the implementation.

### `i32x4_relaxed_trunc_f32x4`

- Signature: `pub fn i32x4_relaxed_trunc_f32x4(a: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_trunc_f32x4_s`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `i32x4_trunc_sat_f32x4(a)` converts the `f32` lanes
of `a` to signed 32-bit integers.

Values which don't fit in 32-bit integers or are NaN may have the same
result as `i32x4_trunc_sat_f32x4` or may return `i32::MIN`.

### `u32x4_relaxed_trunc_f32x4`

- Signature: `pub fn u32x4_relaxed_trunc_f32x4(a: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_trunc_f32x4_u`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `u32x4_trunc_sat_f32x4(a)` converts the `f32` lanes
of `a` to unsigned 32-bit integers.

Values which don't fit in 32-bit unsigned integers or are NaN may have the
same result as `u32x4_trunc_sat_f32x4` or may return `u32::MAX`.

### `i32x4_relaxed_trunc_f64x2_zero`

- Signature: `pub fn i32x4_relaxed_trunc_f64x2_zero(a: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_trunc_f64x2_s_zero`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `i32x4_trunc_sat_f64x2_zero(a)` converts the `f64`
lanes of `a` to signed 32-bit integers and the upper two lanes are zero.

Values which don't fit in 32-bit integers or are NaN may have the same
result as `i32x4_trunc_sat_f32x4` or may return `i32::MIN`.

### `u32x4_relaxed_trunc_f64x2_zero`

- Signature: `pub fn u32x4_relaxed_trunc_f64x2_zero(a: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_trunc_f64x2_u_zero`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `u32x4_trunc_sat_f64x2_zero(a)` converts the `f64`
lanes of `a` to unsigned 32-bit integers and the upper two lanes are zero.

Values which don't fit in 32-bit unsigned integers or are NaN may have the
same result as `u32x4_trunc_sat_f32x4` or may return `u32::MAX`.

### `f32x4_relaxed_madd`

- Signature: `pub fn f32x4_relaxed_madd(a: v128, b: v128, c: v128) -> v128`
- Wasm op / alias: `f32x4.relaxed_madd`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

Computes `a * b + c` with either one rounding or two roundings.

### `f32x4_relaxed_nmadd`

- Signature: `pub fn f32x4_relaxed_nmadd(a: v128, b: v128, c: v128) -> v128`
- Wasm op / alias: `f32x4.relaxed_nmadd`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

Computes `-a * b + c` with either one rounding or two roundings.

### `f64x2_relaxed_madd`

- Signature: `pub fn f64x2_relaxed_madd(a: v128, b: v128, c: v128) -> v128`
- Wasm op / alias: `f64x2.relaxed_madd`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

Computes `a * b + c` with either one rounding or two roundings.

### `f64x2_relaxed_nmadd`

- Signature: `pub fn f64x2_relaxed_nmadd(a: v128, b: v128, c: v128) -> v128`
- Wasm op / alias: `f64x2.relaxed_nmadd`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

Computes `-a * b + c` with either one rounding or two roundings.

### `i8x16_relaxed_laneselect`

- Signature: `pub fn i8x16_relaxed_laneselect(a: v128, b: v128, m: v128) -> v128`
- Wasm op / alias: `i8x16.relaxed_laneselect`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `v128_bitselect` where this either behaves the same as
`v128_bitselect` or the high bit of each lane `m` is inspected and the
corresponding lane of `a` is chosen if the bit is 1 or the lane of `b` is
chosen if it's zero.

If the `m` mask's lanes are either all-one or all-zero then this instruction
is the same as `v128_bitselect`.

### `i16x8_relaxed_laneselect`

- Signature: `pub fn i16x8_relaxed_laneselect(a: v128, b: v128, m: v128) -> v128`
- Wasm op / alias: `i16x8.relaxed_laneselect`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `v128_bitselect` where this either behaves the same as
`v128_bitselect` or the high bit of each lane `m` is inspected and the
corresponding lane of `a` is chosen if the bit is 1 or the lane of `b` is
chosen if it's zero.

If the `m` mask's lanes are either all-one or all-zero then this instruction
is the same as `v128_bitselect`.

### `i32x4_relaxed_laneselect`

- Signature: `pub fn i32x4_relaxed_laneselect(a: v128, b: v128, m: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_laneselect`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `v128_bitselect` where this either behaves the same as
`v128_bitselect` or the high bit of each lane `m` is inspected and the
corresponding lane of `a` is chosen if the bit is 1 or the lane of `b` is
chosen if it's zero.

If the `m` mask's lanes are either all-one or all-zero then this instruction
is the same as `v128_bitselect`.

### `i64x2_relaxed_laneselect`

- Signature: `pub fn i64x2_relaxed_laneselect(a: v128, b: v128, m: v128) -> v128`
- Wasm op / alias: `i64x2.relaxed_laneselect`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `v128_bitselect` where this either behaves the same as
`v128_bitselect` or the high bit of each lane `m` is inspected and the
corresponding lane of `a` is chosen if the bit is 1 or the lane of `b` is
chosen if it's zero.

If the `m` mask's lanes are either all-one or all-zero then this instruction
is the same as `v128_bitselect`.

### `f32x4_relaxed_min`

- Signature: `pub fn f32x4_relaxed_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.relaxed_min`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `f32x4_min` which has implementation-specific behavior
when its operands are NaN or signed zeroes. For more information, see the WebAssembly specification.

### `f32x4_relaxed_max`

- Signature: `pub fn f32x4_relaxed_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `f32x4.relaxed_max`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `f32x4_max` which has implementation-specific behavior
when its operands are NaN or signed zeroes. For more information, see the WebAssembly specification.

### `f64x2_relaxed_min`

- Signature: `pub fn f64x2_relaxed_min(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.relaxed_min`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `f64x2_min` which has implementation-specific behavior
when its operands are NaN or signed zeroes. For more information, see the WebAssembly specification.

### `f64x2_relaxed_max`

- Signature: `pub fn f64x2_relaxed_max(a: v128, b: v128) -> v128`
- Wasm op / alias: `f64x2.relaxed_max`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `f64x2_max` which has implementation-specific behavior
when its operands are NaN or signed zeroes. For more information, see the WebAssembly specification.

### `i16x8_relaxed_q15mulr`

- Signature: `pub fn i16x8_relaxed_q15mulr(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.relaxed_q15mulr_s`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed version of `i16x8_relaxed_q15mulr` where if both lanes are
`i16::MIN` then the result is either `i16::MIN` or `i16::MAX`.

### `i16x8_relaxed_dot_i8x16_i7x16`

- Signature: `pub fn i16x8_relaxed_dot_i8x16_i7x16(a: v128, b: v128) -> v128`
- Wasm op / alias: `i16x8.relaxed_dot_i8x16_i7x16_s`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

A relaxed dot-product instruction.

This instruction will perform pairwise products of the 8-bit values in `a`
and `b` and then accumulate adjacent pairs into 16-bit results producing a
final `i16x8` vector. The bytes of `a` are always interpreted as signed and
the bytes in `b` may be interpreted as signed or unsigned. If the top bit in
`b` isn't set then the value is the same regardless of whether it's signed
or unsigned.

The accumulation into 16-bit values may be saturated on some platforms, and
on other platforms it may wrap-around on overflow.

### `i32x4_relaxed_dot_i8x16_i7x16_add`

- Signature: `pub fn i32x4_relaxed_dot_i8x16_i7x16_add(a: v128, b: v128, c: v128) -> v128`
- Wasm op / alias: `i32x4.relaxed_dot_i8x16_i7x16_add_s`
- Feature: `relaxed-simd`
- Status: stable 1.82.0 (`stdarch_wasm_relaxed_simd`)
- Flags / constraints: -

Source documentation:

Similar to `i16x8_relaxed_dot_i8x16_i7x16` except that the intermediate
`i16x8` result is fed into `i32x4_extadd_pairwise_i16x8` followed by
`i32x4_add` to add the value `c` to the result.

## Public aliases

| Alias | Target | Module | Wasm op / alias | Feature | Status |
| --- | --- | --- | --- | --- | --- |
| `u16x8_load_extend_u8x8` | `i16x8_load_extend_u8x8` | `simd128.rs` | `v128.load8x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_load_extend_u16x4` | `i32x4_load_extend_u16x4` | `simd128.rs` | `v128.load16x4_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_load_extend_u32x2` | `i64x2_load_extend_u32x2` | `simd128.rs` | `v128.load32x2_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_shuffle` | `i8x16_shuffle` | `simd128.rs` | `i8x16.shuffle` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_shuffle` | `i16x8_shuffle` | `simd128.rs` | `i8x16.shuffle` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_shuffle` | `i32x4_shuffle` | `simd128.rs` | `i8x16.shuffle` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_shuffle` | `i64x2_shuffle` | `simd128.rs` | `i8x16.shuffle` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_swizzle` | `i8x16_swizzle` | `simd128.rs` | `i8x16.swizzle` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_eq` | `i8x16_eq` | `simd128.rs` | `i8x16.eq` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_ne` | `i8x16_ne` | `simd128.rs` | `i8x16.ne` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_eq` | `i16x8_eq` | `simd128.rs` | `i16x8.eq` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_ne` | `i16x8_ne` | `simd128.rs` | `i16x8.ne` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_eq` | `i32x4_eq` | `simd128.rs` | `i32x4.eq` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_ne` | `i32x4_ne` | `simd128.rs` | `i32x4.ne` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_eq` | `i64x2_eq` | `simd128.rs` | `i64x2.eq` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_ne` | `i64x2_ne` | `simd128.rs` | `i64x2.ne` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_popcnt` | `i8x16_popcnt` | `simd128.rs` | `i8x16.popcnt` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_all_true` | `i8x16_all_true` | `simd128.rs` | `i8x16.all_true` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_bitmask` | `i8x16_bitmask` | `simd128.rs` | `i8x16.bitmask` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_shl` | `i8x16_shl` | `simd128.rs` | `i8x16.shl` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_add` | `i8x16_add` | `simd128.rs` | `i8x16.add` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_sub` | `i8x16_sub` | `simd128.rs` | `i8x16.sub` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_extadd_pairwise_u8x16` | `i16x8_extadd_pairwise_u8x16` | `simd128.rs` | `i16x8.extadd_pairwise_i8x16_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_all_true` | `i16x8_all_true` | `simd128.rs` | `i16x8.all_true` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_bitmask` | `i16x8_bitmask` | `simd128.rs` | `i16x8.bitmask` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_extend_low_u8x16` | `i16x8_extend_low_u8x16` | `simd128.rs` | `i16x8.extend_low_i8x16_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_extend_high_u8x16` | `i16x8_extend_high_u8x16` | `simd128.rs` | `i16x8.extend_high_i8x16_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_shl` | `i16x8_shl` | `simd128.rs` | `i16x8.shl` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_add` | `i16x8_add` | `simd128.rs` | `i16x8.add` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_sub` | `i16x8_sub` | `simd128.rs` | `i16x8.sub` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_mul` | `i16x8_mul` | `simd128.rs` | `i16x8.mul` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_extmul_low_u8x16` | `i16x8_extmul_low_u8x16` | `simd128.rs` | `i16x8.extmul_low_i8x16_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u16x8_extmul_high_u8x16` | `i16x8_extmul_high_u8x16` | `simd128.rs` | `i16x8.extmul_high_i8x16_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_extadd_pairwise_u16x8` | `i32x4_extadd_pairwise_u16x8` | `simd128.rs` | `i32x4.extadd_pairwise_i16x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_all_true` | `i32x4_all_true` | `simd128.rs` | `i32x4.all_true` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_bitmask` | `i32x4_bitmask` | `simd128.rs` | `i32x4.bitmask` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_extend_low_u16x8` | `i32x4_extend_low_u16x8` | `simd128.rs` | `i32x4.extend_low_i16x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_extend_high_u16x8` | `i32x4_extend_high_u16x8` | `simd128.rs` | `i32x4.extend_high_i16x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_shl` | `i32x4_shl` | `simd128.rs` | `i32x4.shl` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_add` | `i32x4_add` | `simd128.rs` | `i32x4.add` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_sub` | `i32x4_sub` | `simd128.rs` | `i32x4.sub` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_mul` | `i32x4_mul` | `simd128.rs` | `i32x4.mul` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_extmul_low_u16x8` | `i32x4_extmul_low_u16x8` | `simd128.rs` | `i32x4.extmul_low_i16x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u32x4_extmul_high_u16x8` | `i32x4_extmul_high_u16x8` | `simd128.rs` | `i32x4.extmul_high_i16x8_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_all_true` | `i64x2_all_true` | `simd128.rs` | `i64x2.all_true` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_bitmask` | `i64x2_bitmask` | `simd128.rs` | `i64x2.bitmask` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_extend_low_u32x4` | `i64x2_extend_low_u32x4` | `simd128.rs` | `i64x2.extend_low_i32x4_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_extend_high_u32x4` | `i64x2_extend_high_u32x4` | `simd128.rs` | `i64x2.extend_high_i32x4_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_shl` | `i64x2_shl` | `simd128.rs` | `i64x2.shl` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_add` | `i64x2_add` | `simd128.rs` | `i64x2.add` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_sub` | `i64x2_sub` | `simd128.rs` | `i64x2.sub` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_mul` | `i64x2_mul` | `simd128.rs` | `i64x2.mul` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_extmul_low_u32x4` | `i64x2_extmul_low_u32x4` | `simd128.rs` | `i64x2.extmul_low_i32x4_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u64x2_extmul_high_u32x4` | `i64x2_extmul_high_u32x4` | `simd128.rs` | `i64x2.extmul_high_i32x4_u` | `simd128` | stable 1.54.0 (`wasm_simd`) |
| `u8x16_relaxed_swizzle` | `i8x16_relaxed_swizzle` | `relaxed_simd.rs` | `i8x16.relaxed_swizzle` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u8x16_relaxed_laneselect` | `i8x16_relaxed_laneselect` | `relaxed_simd.rs` | `i8x16.relaxed_laneselect` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u16x8_relaxed_laneselect` | `i16x8_relaxed_laneselect` | `relaxed_simd.rs` | `i16x8.relaxed_laneselect` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u32x4_relaxed_laneselect` | `i32x4_relaxed_laneselect` | `relaxed_simd.rs` | `i32x4.relaxed_laneselect` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u64x2_relaxed_laneselect` | `i64x2_relaxed_laneselect` | `relaxed_simd.rs` | `i64x2.relaxed_laneselect` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u16x8_relaxed_q15mulr` | `i16x8_relaxed_q15mulr` | `relaxed_simd.rs` | `i16x8.relaxed_q15mulr_s` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u16x8_relaxed_dot_i8x16_i7x16` | `i16x8_relaxed_dot_i8x16_i7x16` | `relaxed_simd.rs` | `i16x8.relaxed_dot_i8x16_i7x16_s` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |
| `u32x4_relaxed_dot_i8x16_i7x16_add` | `i32x4_relaxed_dot_i8x16_i7x16_add` | `relaxed_simd.rs` | `i32x4.relaxed_dot_i8x16_i7x16_add_s` | `relaxed-simd` | stable 1.82.0 (`stdarch_wasm_relaxed_simd`) |

## Extraction counts

- Public functions: 294
- Public aliases: 62

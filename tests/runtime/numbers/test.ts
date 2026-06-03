//@ [lang]
//@ path = "exports/test$numbers$numbers.ts"

import * as ffi from "../ffi";

let SCALAR: u32 = 0;

export function roundtripU8(a: u8): u8 { return a; }
export function roundtripS8(a: i8): i8 { return a; }
export function roundtripU16(a: u16): u16 { return a; }
export function roundtripS16(a: i16): i16 { return a; }
export function roundtripU32(a: u32): u32 { return a; }
export function roundtripS32(a: i32): i32 { return a; }
export function roundtripU64(a: u64): u64 { return a; }
export function roundtripS64(a: i64): i64 { return a; }
export function roundtripF32(a: f32): f32 { return a; }
export function roundtripF64(a: f64): f64 { return a; }
export function roundtripChar(a: i32): i32 { return a; }

export function setScalar(a: u32): void { SCALAR = a; }
export function getScalar(): u32 { return SCALAR; }

// wasm export wrappers (verbatim from the generated stub; required for asc to
// see the exports the host will call).

export function __exp_0_roundtripU8(a0: i32): i32 { return (<i32>roundtripU8((<u8>a0))); }
export function __exp_0_roundtripS8(a0: i32): i32 { return (<i32>roundtripS8((<i8>a0))); }
export function __exp_0_roundtripU16(a0: i32): i32 { return (<i32>roundtripU16((<u16>a0))); }
export function __exp_0_roundtripS16(a0: i32): i32 { return (<i32>roundtripS16((<i16>a0))); }
export function __exp_0_roundtripU32(a0: i32): i32 { return (<i32>roundtripU32((<u32>a0))); }
export function __exp_0_roundtripS32(a0: i32): i32 { return (<i32>roundtripS32((<i32>a0))); }
export function __exp_0_roundtripU64(a0: i64): i64 { return (<i64>roundtripU64((<u64>a0))); }
export function __exp_0_roundtripS64(a0: i64): i64 { return (<i64>roundtripS64((<i64>a0))); }
export function __exp_0_roundtripF32(a0: f32): f32 { return (<f32>roundtripF32((<f32>a0))); }
export function __exp_0_roundtripF64(a0: f64): f64 { return (<f64>roundtripF64((<f64>a0))); }
export function __exp_0_roundtripChar(a0: i32): i32 { return (<i32>roundtripChar((<i32>a0))); }
export function __exp_0_setScalar(a0: i32): void { setScalar((<u32>a0)); }
export function __exp_0_getScalar(): i32 { return (<i32>getScalar()); }

// silence unused-import warning
// @ts-ignore
const _keepalive = ffi.cabi_realloc;

//@ [lang]
//@ path = "stubs/test$numbers$numbers.ts"

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

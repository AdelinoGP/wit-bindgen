//@ [lang]
//@ path = "world.ts"

import * as ffi from "./ffi";
import * as i_test$numbers$numbers from "./imports/test$numbers$numbers";

function assertEq<T>(actual: T, expected: T): void {
  if (actual != expected) unreachable();
}

export function run(): void {
  assertEq<u8>(i_test$numbers$numbers.roundtripU8(1), 1);
  assertEq<u8>(i_test$numbers$numbers.roundtripU8(u8.MIN_VALUE), u8.MIN_VALUE);
  assertEq<u8>(i_test$numbers$numbers.roundtripU8(u8.MAX_VALUE), u8.MAX_VALUE);

  assertEq<i8>(i_test$numbers$numbers.roundtripS8(1), 1);
  assertEq<i8>(i_test$numbers$numbers.roundtripS8(i8.MIN_VALUE), i8.MIN_VALUE);
  assertEq<i8>(i_test$numbers$numbers.roundtripS8(i8.MAX_VALUE), i8.MAX_VALUE);

  assertEq<u16>(i_test$numbers$numbers.roundtripU16(1), 1);
  assertEq<u16>(i_test$numbers$numbers.roundtripU16(u16.MIN_VALUE), u16.MIN_VALUE);
  assertEq<u16>(i_test$numbers$numbers.roundtripU16(u16.MAX_VALUE), u16.MAX_VALUE);

  assertEq<i16>(i_test$numbers$numbers.roundtripS16(1), 1);
  assertEq<i16>(i_test$numbers$numbers.roundtripS16(i16.MIN_VALUE), i16.MIN_VALUE);
  assertEq<i16>(i_test$numbers$numbers.roundtripS16(i16.MAX_VALUE), i16.MAX_VALUE);

  assertEq<u32>(i_test$numbers$numbers.roundtripU32(1), 1);
  assertEq<u32>(i_test$numbers$numbers.roundtripU32(u32.MIN_VALUE), u32.MIN_VALUE);
  assertEq<u32>(i_test$numbers$numbers.roundtripU32(u32.MAX_VALUE), u32.MAX_VALUE);

  assertEq<i32>(i_test$numbers$numbers.roundtripS32(1), 1);
  assertEq<i32>(i_test$numbers$numbers.roundtripS32(i32.MIN_VALUE), i32.MIN_VALUE);
  assertEq<i32>(i_test$numbers$numbers.roundtripS32(i32.MAX_VALUE), i32.MAX_VALUE);

  assertEq<u64>(i_test$numbers$numbers.roundtripU64(1), 1);
  assertEq<u64>(i_test$numbers$numbers.roundtripU64(u64.MAX_VALUE), u64.MAX_VALUE);

  assertEq<i64>(i_test$numbers$numbers.roundtripS64(1), 1);
  assertEq<i64>(i_test$numbers$numbers.roundtripS64(i64.MIN_VALUE), i64.MIN_VALUE);
  assertEq<i64>(i_test$numbers$numbers.roundtripS64(i64.MAX_VALUE), i64.MAX_VALUE);

  assertEq<f32>(i_test$numbers$numbers.roundtripF32(1.0), 1.0);
  assertEq<f64>(i_test$numbers$numbers.roundtripF64(1.0), 1.0);

  assertEq<i32>(i_test$numbers$numbers.roundtripChar(0x61), 0x61); // 'a'
  assertEq<i32>(i_test$numbers$numbers.roundtripChar(0x20), 0x20); // ' '

  i_test$numbers$numbers.setScalar(2);
  assertEq<u32>(i_test$numbers$numbers.getScalar(), 2);
  i_test$numbers$numbers.setScalar(4);
  assertEq<u32>(i_test$numbers$numbers.getScalar(), 4);
}

// wasm export wrapper — the generator emits this name for world-level `run`.
export function __exp_18446744073709551615_run(): void { run(); }

// keep ffi alive
// @ts-ignore
const _keepalive = ffi.cabi_realloc;

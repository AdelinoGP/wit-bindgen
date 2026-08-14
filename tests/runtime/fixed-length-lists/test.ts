//@ [lang]
//@ path = "stubs/test$fixed_length_lists$to_test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$fixed_length_lists$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function assertU32(a: StaticArray<u32>, expected: u32[]): void {
  assert(a.length == expected.length);
  for (let i = 0; i < a.length; i++) assert(a[i] == expected[i]);
}

function assertI32(a: StaticArray<i32>, expected: i32[]): void {
  assert(a.length == expected.length);
  for (let i = 0; i < a.length; i++) assert(a[i] == expected[i]);
}

export function listParam(a: StaticArray<u32>): void {
  assertU32(a, [1, 2, 3, 4]);
}

export function listParam2(a: StaticArray<StaticArray<u32>>): void {
  assert(a.length == 2);
  assertU32(a[0], [1, 2]);
  assertU32(a[1], [3, 4]);
}

export function listParam3(a: StaticArray<i32>): void {
  assertI32(a, [
    -1, 2, -3, 4, -5, 6, -7, 8, -9, 10, -11, 12, -13, 14, -15, 16, -17, 18, -19, 20,
  ]);
}

export function listResult(): StaticArray<u8> {
  const out = new StaticArray<u8>(8);
  const values: u8[] = [0x30, 0x31, 0x41, 0x42, 0x61, 0x62, 128, 255];
  for (let i = 0; i < 8; i++) out[i] = values[i];
  return out;
}

export function listMinmax16(
  a: StaticArray<u16>,
  b: StaticArray<i16>,
): ffi.Tuple2<StaticArray<u16>, StaticArray<i16>> {
  return new ffi.Tuple2<StaticArray<u16>, StaticArray<i16>>(a, b);
}

export function listMinmaxFloat(
  a: StaticArray<f32>,
  b: StaticArray<f64>,
): ffi.Tuple2<StaticArray<f32>, StaticArray<f64>> {
  return new ffi.Tuple2<StaticArray<f32>, StaticArray<f64>>(a, b);
}

export function listRoundtrip(a: StaticArray<u8>): StaticArray<u8> {
  return a;
}

export function nestedRoundtrip(
  a: StaticArray<StaticArray<u32>>,
  b: StaticArray<StaticArray<i32>>,
): ffi.Tuple2<StaticArray<StaticArray<u32>>, StaticArray<StaticArray<i32>>> {
  return new ffi.Tuple2<StaticArray<StaticArray<u32>>, StaticArray<StaticArray<i32>>>(a, b);
}

export function largeRoundtrip(
  a: StaticArray<StaticArray<u32>>,
  b: StaticArray<StaticArray<i32>>,
): ffi.Tuple2<StaticArray<StaticArray<u32>>, StaticArray<StaticArray<i32>>> {
  return new ffi.Tuple2<StaticArray<StaticArray<u32>>, StaticArray<StaticArray<i32>>>(a, b);
}

export function nightmareOnCpp(a: StaticArray<E.Nested>): StaticArray<E.Nested> {
  return a;
}

//@ [lang]
//@ path = "stubs/test$records$to_test.ts"

import * as ffi from "../ffi";
import * as e_test$records$to_test from "../exports/test$records$to_test";

export function multipleResults(): ffi.Tuple2<u8, u16> {
  return new ffi.Tuple2<u8, u16>(4, 5);
}

export function swapTuple(a: ffi.Tuple2<u8, u32>): ffi.Tuple2<u32, u8> {
  return new ffi.Tuple2<u32, u8>(a._1, a._0);
}

export function roundtripFlags1(
  a: e_test$records$to_test.F1,
): e_test$records$to_test.F1 {
  return a;
}

export function roundtripFlags2(
  a: e_test$records$to_test.F2,
): e_test$records$to_test.F2 {
  return a;
}

export function roundtripFlags3(
  a: e_test$records$to_test.Flag8,
  b: e_test$records$to_test.Flag16,
  c: e_test$records$to_test.Flag32,
): ffi.Tuple3<
  e_test$records$to_test.Flag8,
  e_test$records$to_test.Flag16,
  e_test$records$to_test.Flag32
> {
  return new ffi.Tuple3<
    e_test$records$to_test.Flag8,
    e_test$records$to_test.Flag16,
    e_test$records$to_test.Flag32
  >(a, b, c);
}

export function roundtripRecord1(
  a: e_test$records$to_test.R1,
): e_test$records$to_test.R1 {
  return a;
}

export function tuple1(a: ffi.Tuple1<u8>): ffi.Tuple1<u8> {
  return new ffi.Tuple1<u8>(a._0);
}

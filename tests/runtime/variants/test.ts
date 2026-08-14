//@ [lang]
//@ path = "stubs/test$variants$to_test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$variants$to_test";

export function roundtripOption(a: ffi.Option<f32>): ffi.Option<u8> {
  return new ffi.Option<u8>(a.tag, <u8>a.value);
}

export function roundtripResult(a: ffi.Result<u32, f32>): ffi.Result<f64, u8> {
  if (a.isOk()) return new ffi.Result<f64, u8>(0, <f64>a.okValue, 0);
  return new ffi.Result<f64, u8>(1, 0.0, <u8>a.errValue);
}

export function roundtripEnum(a: E.E1): E.E1 {
  return a;
}

export function invertBool(a: bool): bool {
  return !a;
}

export function variantCasts(
  a: ffi.Tuple6<E.C1, E.C2, E.C3, E.C4, E.C5, E.C6>,
): ffi.Tuple6<E.C1, E.C2, E.C3, E.C4, E.C5, E.C6> {
  return a;
}

export function variantZeros(
  a: ffi.Tuple4<E.Z1, E.Z2, E.Z3, E.Z4>,
): ffi.Tuple4<E.Z1, E.Z2, E.Z3, E.Z4> {
  return a;
}

export function variantTypedefs(a: ffi.Option<u32>, b: bool, c: ffi.Result<u32, i32>): void {}

export function variantEnums(
  a: bool,
  b: ffi.Result<i32, i32>,
  c: E.MyErrno,
): ffi.Tuple3<bool, ffi.Result<i32, i32>, E.MyErrno> {
  return new ffi.Tuple3<bool, ffi.Result<i32, i32>, E.MyErrno>(a, b, c);
}

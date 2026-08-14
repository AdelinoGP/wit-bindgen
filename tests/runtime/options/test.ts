//@ [lang]
//@ path = "stubs/test$options$to_test.ts"

import * as ffi from "../ffi";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function optionNoneParam(a: ffi.Option<string>): void {
  assert(a.isNone());
}

export function optionSomeParam(a: ffi.Option<string>): void {
  assert(a.isSome());
  assert(a.value == "foo");
}

export function optionNoneResult(): ffi.Option<string> {
  return new ffi.Option<string>(0, "");
}

export function optionSomeResult(): ffi.Option<string> {
  return new ffi.Option<string>(1, "foo");
}

export function optionRoundtrip(a: ffi.Option<string>): ffi.Option<string> {
  return a;
}

export function doubleOptionRoundtrip(
  a: ffi.Option<ffi.Option<u32>>,
): ffi.Option<ffi.Option<u32>> {
  return a;
}

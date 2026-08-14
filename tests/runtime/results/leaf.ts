//@ [lang]
//@ path = "stubs/test$results$test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$results$test";

export function stringError(a: f32): ffi.Result<f32, string> {
  if (a == 0.0) return new ffi.Result<f32, string>(1, 0.0, "zero");
  return new ffi.Result<f32, string>(0, a, "");
}

export function enumError(a: f32): ffi.Result<f32, E.E> {
  if (a == 0.0) return new ffi.Result<f32, E.E>(1, 0.0, E.E.A);
  return new ffi.Result<f32, E.E>(0, a, E.E.A);
}

export function recordError(a: f32): ffi.Result<f32, E.E2> {
  if (a == 0.0) return new ffi.Result<f32, E.E2>(1, 0.0, new E.E2(420, 0));
  if (a == 1.0) return new ffi.Result<f32, E.E2>(1, 0.0, new E.E2(77, 2));
  return new ffi.Result<f32, E.E2>(0, a, new E.E2(0, 0));
}

export function variantError(a: f32): ffi.Result<f32, E.E3> {
  const ok = new E.E3_E1(E.E.A);
  if (a == 0.0) {
    return new ffi.Result<f32, E.E3>(1, 0.0, new E.E3_E2(new E.E2(420, 0)));
  }
  if (a == 1.0) return new ffi.Result<f32, E.E3>(1, 0.0, new E.E3_E1(E.E.B));
  if (a == 2.0) return new ffi.Result<f32, E.E3>(1, 0.0, new E.E3_E1(E.E.C));
  return new ffi.Result<f32, E.E3>(0, a, ok);
}

export function emptyError(a: u32): ffi.Result<u32, i32> {
  if (a == 0) return new ffi.Result<u32, i32>(1, 0, 0);
  if (a == 1) return new ffi.Result<u32, i32>(0, 42, 0);
  return new ffi.Result<u32, i32>(0, a, 0);
}

export function doubleError(a: u32): ffi.Result<ffi.Result<i32, string>, string> {
  if (a == 0) {
    return new ffi.Result<ffi.Result<i32, string>, string>(
      0,
      new ffi.Result<i32, string>(0, 0, ""),
      "",
    );
  }
  if (a == 1) {
    return new ffi.Result<ffi.Result<i32, string>, string>(
      0,
      new ffi.Result<i32, string>(1, 0, "one"),
      "",
    );
  }
  return new ffi.Result<ffi.Result<i32, string>, string>(
    1,
    new ffi.Result<i32, string>(0, 0, ""),
    "two",
  );
}

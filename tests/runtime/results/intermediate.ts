//@ [lang]
//@ path = "stubs/test$results$test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$results$test";
import * as I from "../imports/test$results$test";

// The imported and exported views of these types are distinct, so every value
// is rebuilt on the way through rather than forwarded.

export function stringError(a: f32): ffi.Result<f32, string> {
  return I.stringError(a);
}

export function enumError(a: f32): ffi.Result<f32, E.E> {
  const inner = I.enumError(a);
  if (inner.isOk()) return new ffi.Result<f32, E.E>(0, inner.okValue, E.E.A);
  return new ffi.Result<f32, E.E>(1, 0.0, <E.E>(<i32>inner.errValue));
}

export function recordError(a: f32): ffi.Result<f32, E.E2> {
  const inner = I.recordError(a);
  if (inner.isOk()) {
    return new ffi.Result<f32, E.E2>(0, inner.okValue, new E.E2(0, 0));
  }
  return new ffi.Result<f32, E.E2>(
    1,
    0.0,
    new E.E2(inner.errValue.line, inner.errValue.column),
  );
}

export function variantError(a: f32): ffi.Result<f32, E.E3> {
  const inner = I.variantError(a);
  const placeholder = new E.E3_E1(E.E.A);
  if (inner.isOk()) return new ffi.Result<f32, E.E3>(0, inner.okValue, placeholder);
  const err = inner.errValue;
  if (err.tag == 0) {
    const e1 = changetype<I.E3_E1>(err);
    return new ffi.Result<f32, E.E3>(1, 0.0, new E.E3_E1(<E.E>(<i32>e1.value)));
  }
  const e2 = changetype<I.E3_E2>(err);
  return new ffi.Result<f32, E.E3>(
    1,
    0.0,
    new E.E3_E2(new E.E2(e2.value.line, e2.value.column)),
  );
}

export function emptyError(a: u32): ffi.Result<u32, i32> {
  return I.emptyError(a);
}

export function doubleError(a: u32): ffi.Result<ffi.Result<i32, string>, string> {
  return I.doubleError(a);
}

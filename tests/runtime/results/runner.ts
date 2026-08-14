//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$results$test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  let s = I.stringError(0.0);
  assert(s.isErr() && s.errValue == "zero");
  s = I.stringError(1.0);
  assert(s.isOk() && s.okValue == 1.0);

  let e = I.enumError(0.0);
  assert(e.isErr() && e.errValue == I.E.A);
  e = I.enumError(1.0);
  assert(e.isOk() && e.okValue == 1.0);

  let r = I.recordError(0.0);
  assert(r.isErr() && r.errValue.line == 420 && r.errValue.column == 0);
  r = I.recordError(1.0);
  assert(r.isErr() && r.errValue.line == 77 && r.errValue.column == 2);
  assert(I.recordError(2.0).isOk());

  let v = I.variantError(0.0);
  assert(v.isErr() && v.errValue.tag == 1);
  const e2 = changetype<I.E3_E2>(v.errValue);
  assert(e2.value.line == 420 && e2.value.column == 0);
  v = I.variantError(1.0);
  assert(v.isErr() && v.errValue.tag == 0);
  assert(changetype<I.E3_E1>(v.errValue).value == I.E.B);
  v = I.variantError(2.0);
  assert(v.isErr() && v.errValue.tag == 0);
  assert(changetype<I.E3_E1>(v.errValue).value == I.E.C);

  let m = I.emptyError(0);
  assert(m.isErr());
  m = I.emptyError(1);
  assert(m.isOk() && m.okValue == 42);
  m = I.emptyError(2);
  assert(m.isOk() && m.okValue == 2);

  let d = I.doubleError(0);
  assert(d.isOk() && d.okValue.isOk());
  d = I.doubleError(1);
  assert(d.isOk() && d.okValue.isErr() && d.okValue.errValue == "one");
  d = I.doubleError(2);
  assert(d.isErr() && d.errValue == "two");
}

//@ [lang]
//@ path = "stubs/test$resource_aggregates$to_test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$resource_aggregates$to_test";

export class Thing {
  constructor(public value: u32) {}
  __onDrop(): void {}
}

export function constructorThing(v: u32): Thing {
  return new Thing(v + 1);
}

export function foo(
  r1: E.R1,
  r2: E.R2,
  r3: E.R3,
  t1: ffi.Tuple2<Thing, E.R1>,
  t2: ffi.Tuple1<Thing>,
  v1: E.V1,
  v2: E.V2,
  l1: Array<Thing>,
  l2: Array<Thing>,
  o1: ffi.Option<Thing>,
  o2: ffi.Option<Thing>,
  result1: ffi.Result<Thing, i32>,
  result2: ffi.Result<Thing, i32>,
): u32 {
  let total: u32 = 0;
  total += r1.thing.value;
  total += r2.thing.value;
  total += r3.thing1.value;
  total += r3.thing2.value;
  total += t1._0.value;
  total += t1._1.thing.value;
  total += t2._0.value;
  total += changetype<E.V1_Thing>(v1).value.value;
  total += changetype<E.V2_Thing>(v2).value.value;
  for (let i = 0; i < l1.length; i++) total += l1[i].value;
  for (let i = 0; i < l2.length; i++) total += l2[i].value;
  if (o1.isSome()) total += o1.value.value;
  if (o2.isSome()) total += o2.value.value;
  if (result1.isOk()) total += result1.okValue.value;
  if (result2.isOk()) total += result2.okValue.value;
  return total + 3;
}

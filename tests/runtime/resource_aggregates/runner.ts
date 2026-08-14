//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as I from "../imports/test$resource_aggregates$to_test";

function list(things: I.Thing[]): Array<I.Thing> {
  const out = new Array<I.Thing>(things.length);
  for (let i = 0; i < things.length; i++) out[i] = things[i];
  return out;
}

export function run(): void {
  // Owned and borrowed handles reach the callee through every aggregate the
  // canonical ABI has: records, tuples, variants, lists, options, and results.
  const total = I.foo(
    new I.R1(I.constructorThing(0)),
    new I.R2(I.constructorThing(1)),
    new I.R3(I.constructorThing(2), I.constructorThing(3)),
    new ffi.Tuple2<I.Thing, I.R1>(
      I.constructorThing(4),
      new I.R1(I.constructorThing(5)),
    ),
    new ffi.Tuple1<I.Thing>(I.constructorThing(6)),
    new I.V1_Thing(I.constructorThing(7)),
    new I.V2_Thing(I.constructorThing(8)),
    list([I.constructorThing(9), I.constructorThing(10)]),
    list([I.constructorThing(11), I.constructorThing(12)]),
    new ffi.Option<I.Thing>(1, I.constructorThing(13)),
    new ffi.Option<I.Thing>(1, I.constructorThing(14)),
    new ffi.Result<I.Thing, i32>(0, I.constructorThing(15), 0),
    new ffi.Result<I.Thing, i32>(0, I.constructorThing(16), 0),
  );

  // The callee adds one to each of 0..=16 and then three.
  let expected: u32 = 3;
  for (let i: u32 = 0; i < 17; i++) expected += i + 1;
  if (total != expected) unreachable();
}

//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as i_test$records$to_test from "../imports/test$records$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function roundtripF1(bits: u8): void {
  const out = i_test$records$to_test.roundtripFlags1(
    new i_test$records$to_test.F1(bits),
  );
  assert(out.bits == bits);
}

function roundtripF2(bits: u8): void {
  const out = i_test$records$to_test.roundtripFlags2(
    new i_test$records$to_test.F2(bits),
  );
  assert(out.bits == bits);
}

export function run(): void {
  const multiple = i_test$records$to_test.multipleResults();
  assert(multiple._0 == 4);
  assert(multiple._1 == 5);

  const swapped = i_test$records$to_test.swapTuple(new ffi.Tuple2<u8, u32>(1, 2));
  assert(swapped._0 == 2);
  assert(swapped._1 == 1);

  roundtripF1(i_test$records$to_test.F1.A);
  roundtripF1(0);
  roundtripF1(i_test$records$to_test.F1.B);
  roundtripF1(i_test$records$to_test.F1.A | i_test$records$to_test.F1.B);

  roundtripF2(i_test$records$to_test.F2.C);
  roundtripF2(0);
  roundtripF2(i_test$records$to_test.F2.D);
  roundtripF2(i_test$records$to_test.F2.C | i_test$records$to_test.F2.E);

  const wide = i_test$records$to_test.roundtripFlags3(
    new i_test$records$to_test.Flag8(i_test$records$to_test.Flag8.B0),
    new i_test$records$to_test.Flag16(i_test$records$to_test.Flag16.B1),
    new i_test$records$to_test.Flag32(i_test$records$to_test.Flag32.B2),
  );
  assert(wide._0.bits == i_test$records$to_test.Flag8.B0);
  assert(wide._1.bits == i_test$records$to_test.Flag16.B1);
  assert(wide._2.bits == i_test$records$to_test.Flag32.B2);

  let record = i_test$records$to_test.roundtripRecord1(
    new i_test$records$to_test.R1(8, new i_test$records$to_test.F1(0)),
  );
  assert(record.a == 8);
  assert(record.b.bits == 0);

  record = i_test$records$to_test.roundtripRecord1(
    new i_test$records$to_test.R1(
      0,
      new i_test$records$to_test.F1(
        i_test$records$to_test.F1.A | i_test$records$to_test.F1.B,
      ),
    ),
  );
  assert(record.a == 0);
  assert(record.b.bits == (i_test$records$to_test.F1.A | i_test$records$to_test.F1.B));

  assert(i_test$records$to_test.tuple1(new ffi.Tuple1<u8>(1))._0 == 1);
}

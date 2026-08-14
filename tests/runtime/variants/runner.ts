//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as I from "../imports/test$variants$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  let opt = I.roundtripOption(new ffi.Option<f32>(1, 1.0));
  assert(opt.isSome() && opt.value == 1);
  opt = I.roundtripOption(new ffi.Option<f32>(0, 0.0));
  assert(opt.isNone());
  opt = I.roundtripOption(new ffi.Option<f32>(1, 2.0));
  assert(opt.isSome() && opt.value == 2);

  let res = I.roundtripResult(new ffi.Result<u32, f32>(0, 2, 0.0));
  assert(res.isOk() && res.okValue == 2.0);
  res = I.roundtripResult(new ffi.Result<u32, f32>(0, 4, 0.0));
  assert(res.isOk() && res.okValue == 4.0);
  res = I.roundtripResult(new ffi.Result<u32, f32>(1, 0, 5.3));
  assert(res.isErr() && res.errValue == 5);

  assert(I.roundtripEnum(I.E1.A) == I.E1.A);
  assert(I.roundtripEnum(I.E1.B) == I.E1.B);

  assert(I.invertBool(true) == false);
  assert(I.invertBool(false) == true);

  // First arm of every cast variant.
  let casts = I.variantCasts(
    new ffi.Tuple6<I.C1, I.C2, I.C3, I.C4, I.C5, I.C6>(
      new I.C1_A(1),
      new I.C2_A(2),
      new I.C3_A(3),
      new I.C4_A(4),
      new I.C5_A(5),
      new I.C6_A(6.0),
    ),
  );
  assert(casts._0.tag == 0 && changetype<I.C1_A>(casts._0).value == 1);
  assert(casts._1.tag == 0 && changetype<I.C2_A>(casts._1).value == 2);
  assert(casts._2.tag == 0 && changetype<I.C3_A>(casts._2).value == 3);
  assert(casts._3.tag == 0 && changetype<I.C4_A>(casts._3).value == 4);
  assert(casts._4.tag == 0 && changetype<I.C5_A>(casts._4).value == 5);
  assert(casts._5.tag == 0 && changetype<I.C6_A>(casts._5).value == 6.0);

  // Second arm, where each payload is a wider or differently-typed value.
  casts = I.variantCasts(
    new ffi.Tuple6<I.C1, I.C2, I.C3, I.C4, I.C5, I.C6>(
      new I.C1_B(1),
      new I.C2_B(2.0),
      new I.C3_B(3.0),
      new I.C4_B(4.0),
      new I.C5_B(5.0),
      new I.C6_B(6.0),
    ),
  );
  assert(casts._0.tag == 1 && changetype<I.C1_B>(casts._0).value == 1);
  assert(casts._1.tag == 1 && changetype<I.C2_B>(casts._1).value == 2.0);
  assert(casts._2.tag == 1 && changetype<I.C3_B>(casts._2).value == 3.0);
  assert(casts._3.tag == 1 && changetype<I.C4_B>(casts._3).value == 4.0);
  assert(casts._4.tag == 1 && changetype<I.C5_B>(casts._4).value == 5.0);
  assert(casts._5.tag == 1 && changetype<I.C6_B>(casts._5).value == 6.0);

  let zeros = I.variantZeros(
    new ffi.Tuple4<I.Z1, I.Z2, I.Z3, I.Z4>(
      new I.Z1_A(1),
      new I.Z2_A(2),
      new I.Z3_A(3.0),
      new I.Z4_A(4.0),
    ),
  );
  assert(zeros._0.tag == 0 && changetype<I.Z1_A>(zeros._0).value == 1);
  assert(zeros._1.tag == 0 && changetype<I.Z2_A>(zeros._1).value == 2);
  assert(zeros._2.tag == 0 && changetype<I.Z3_A>(zeros._2).value == 3.0);
  assert(zeros._3.tag == 0 && changetype<I.Z4_A>(zeros._3).value == 4.0);

  // The payload-less arm: the canonical representation still carries the
  // payload slot, which must be ignored rather than lifted.
  zeros = I.variantZeros(
    new ffi.Tuple4<I.Z1, I.Z2, I.Z3, I.Z4>(
      new I.Z1_B(),
      new I.Z2_B(),
      new I.Z3_B(),
      new I.Z4_B(),
    ),
  );
  assert(zeros._0.tag == 1);
  assert(zeros._1.tag == 1);
  assert(zeros._2.tag == 1);
  assert(zeros._3.tag == 1);

  I.variantTypedefs(
    new ffi.Option<u32>(0, 0),
    false,
    new ffi.Result<u32, i32>(1, 0, 0),
  );

  const enums = I.variantEnums(
    true,
    new ffi.Result<i32, i32>(0, 0, 0),
    I.MyErrno.Success,
  );
  assert(enums._0 == true);
  assert(enums._1.isOk());
  assert(enums._2 == I.MyErrno.Success);
}

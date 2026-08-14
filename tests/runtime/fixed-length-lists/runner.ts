//@ wasmtime-flags = '-Wcomponent-model-fixed-length-lists'
//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$fixed_length_lists$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function u32s(values: u32[]): StaticArray<u32> {
  const out = new StaticArray<u32>(values.length);
  for (let i = 0; i < values.length; i++) out[i] = values[i];
  return out;
}

function i32s(values: i32[]): StaticArray<i32> {
  const out = new StaticArray<i32>(values.length);
  for (let i = 0; i < values.length; i++) out[i] = values[i];
  return out;
}

function assertU32(a: StaticArray<u32>, expected: u32[]): void {
  assert(a.length == expected.length);
  for (let i = 0; i < a.length; i++) assert(a[i] == expected[i]);
}

function assertI32(a: StaticArray<i32>, expected: i32[]): void {
  assert(a.length == expected.length);
  for (let i = 0; i < a.length; i++) assert(a[i] == expected[i]);
}

export function run(): void {
  I.listParam(u32s([1, 2, 3, 4]));

  const nested2 = new StaticArray<StaticArray<u32>>(2);
  nested2[0] = u32s([1, 2]);
  nested2[1] = u32s([3, 4]);
  I.listParam2(nested2);

  I.listParam3(
    i32s([-1, 2, -3, 4, -5, 6, -7, 8, -9, 10, -11, 12, -13, 14, -15, 16, -17, 18, -19, 20]),
  );

  const result = I.listResult();
  const expected: u8[] = [0x30, 0x31, 0x41, 0x42, 0x61, 0x62, 128, 255];
  assert(result.length == 8);
  for (let i = 0; i < 8; i++) assert(result[i] == expected[i]);

  const u16in = new StaticArray<u16>(4);
  const u16values: u16[] = [0, 1024, 32768, 65535];
  for (let i = 0; i < 4; i++) u16in[i] = u16values[i];
  const i16in = new StaticArray<i16>(4);
  const i16values: i16[] = [1, 2048, -32767, -2];
  for (let i = 0; i < 4; i++) i16in[i] = i16values[i];
  const minmax16 = I.listMinmax16(u16in, i16in);
  for (let i = 0; i < 4; i++) {
    assert(minmax16._0[i] == u16values[i]);
    assert(minmax16._1[i] == i16values[i]);
  }

  const f32in = new StaticArray<f32>(2);
  f32in[0] = 2.0;
  f32in[1] = -42.0;
  const f64in = new StaticArray<f64>(2);
  f64in[0] = 0.25;
  f64in[1] = -0.125;
  const minmaxFloat = I.listMinmaxFloat(f32in, f64in);
  assert(minmaxFloat._0[0] == 2.0 && minmaxFloat._0[1] == -42.0);
  assert(minmaxFloat._1[0] == 0.25 && minmaxFloat._1[1] == -0.125);

  const bytes = new StaticArray<u8>(12);
  const byteValues: u8[] = [0x61, 0x62, 0x63, 0x64, 0, 1, 2, 3, 0x41, 0x42, 0x59, 0x5a];
  for (let i = 0; i < 12; i++) bytes[i] = byteValues[i];
  const roundtripped = I.listRoundtrip(bytes);
  for (let i = 0; i < 12; i++) assert(roundtripped[i] == byteValues[i]);

  const nestedA = new StaticArray<StaticArray<u32>>(2);
  nestedA[0] = u32s([1, 5]);
  nestedA[1] = u32s([42, 1000000]);
  const nestedB = new StaticArray<StaticArray<i32>>(2);
  nestedB[0] = i32s([-1, 3]);
  nestedB[1] = i32s([-2000000, 4711]);
  const nestedOut = I.nestedRoundtrip(nestedA, nestedB);
  assertU32(nestedOut._0[0], [1, 5]);
  assertU32(nestedOut._0[1], [42, 1000000]);
  assertI32(nestedOut._1[0], [-1, 3]);
  assertI32(nestedOut._1[1], [-2000000, 4711]);

  const largeB = new StaticArray<StaticArray<i32>>(4);
  largeB[0] = i32s([-1, 3, -2, 4]);
  largeB[1] = i32s([-2000000, 4711, 99999, -5]);
  largeB[2] = i32s([-6, 7, 8, -9]);
  largeB[3] = i32s([50, -5, 500, -5000]);
  const largeOut = I.largeRoundtrip(nestedA, largeB);
  assertU32(largeOut._0[0], [1, 5]);
  assertU32(largeOut._0[1], [42, 1000000]);
  assertI32(largeOut._1[0], [-1, 3, -2, 4]);
  assertI32(largeOut._1[1], [-2000000, 4711, 99999, -5]);
  assertI32(largeOut._1[2], [-6, 7, 8, -9]);
  assertI32(largeOut._1[3], [50, -5, 500, -5000]);

  const nightmare = new StaticArray<I.Nested>(2);
  nightmare[0] = new I.Nested(i32s([1, -1]));
  nightmare[1] = new I.Nested(i32s([2, -2]));
  const nightmareOut = I.nightmareOnCpp(nightmare);
  assertI32(nightmareOut[0].l, [1, -1]);
  assertI32(nightmareOut[1].l, [2, -2]);
}

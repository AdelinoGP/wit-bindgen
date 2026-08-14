//@ wasmtime-flags = '-Wcomponent-model-map'
//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as I from "../imports/test$maps$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function bytes(values: u8[]): Uint8Array {
  const out = new Uint8Array(values.length);
  for (let i = 0; i < values.length; i++) out[i] = values[i];
  return out;
}

function assertBytes(actual: Uint8Array, expected: u8[]): void {
  assert(actual.length == expected.length);
  for (let i = 0; i < actual.length; i++) assert(actual[i] == expected[i]);
}

export function run(): void {
  // Duplicate keys: the later insert wins, and the shadowed entry must not
  // come back across the ABI.
  const names = new Map<u32, string>();
  names.set(1, "one");
  names.set(1, "uno");
  names.set(2, "two");
  const idsByName = I.namedRoundtrip(names);
  assert(idsByName.get("uno") == 1);
  assert(idsByName.get("two") == 2);
  assert(!idsByName.has("one"));

  const bytesInput = new Map<string, Uint8Array>();
  bytesInput.set("hello", bytes([0x77, 0x6f, 0x72, 0x6c, 0x64]));
  bytesInput.set("bin", bytes([0, 1, 2]));
  const bytesOut = I.bytesRoundtrip(bytesInput);
  assertBytes(bytesOut.get("hello"), [0x77, 0x6f, 0x72, 0x6c, 0x64]);
  assertBytes(bytesOut.get("bin"), [0, 1, 2]);

  assert(I.emptyRoundtrip(new Map<u32, string>()).size == 0);

  const options = new Map<string, ffi.Option<u32>>();
  options.set("some", new ffi.Option<u32>(1, 42));
  options.set("none", new ffi.Option<u32>(0, 0));
  const optionsOut = I.optionRoundtrip(options);
  assert(optionsOut.size == 2);
  assert(optionsOut.get("some").isSome() && optionsOut.get("some").value == 42);
  assert(optionsOut.get("none").isNone());

  const values = new Map<u32, string>();
  values.set(10, "ten");
  values.set(20, "twenty");
  const record = I.recordRoundtrip(new I.LabeledEntry("test-label", values));
  assert(record.label == "test-label");
  assert(record.values.size == 2);
  assert(record.values.get(10) == "ten");
  assert(record.values.get(20) == "twenty");

  const inline = new Map<u32, string>();
  inline.set(1, "one");
  inline.set(2, "two");
  const inlineOut = I.inlineRoundtrip(inline);
  assert(inlineOut.size == 2);
  assert(inlineOut.get("one") == 1);
  assert(inlineOut.get("two") == 2);

  // 100 entries: the entry buffer is far past anything the flat paths cover.
  const large = new Map<u32, string>();
  for (let i: u32 = 0; i < 100; i++) large.set(i, "value-" + i.toString());
  const largeOut = I.largeRoundtrip(large);
  assert(largeOut.size == 100);
  for (let i: u32 = 0; i < 100; i++) {
    assert(largeOut.get(i) == "value-" + i.toString());
  }

  const multiBytes = new Map<string, Uint8Array>();
  multiBytes.set("key", bytes([42]));
  const multi = I.multiParamRoundtrip(inline, multiBytes);
  assert(multi._0.size == 2);
  assert(multi._0.get("one") == 1);
  assert(multi._0.get("two") == 2);
  assert(multi._1.size == 1);
  assertBytes(multi._1.get("key"), [42]);

  const innerA = new Map<u32, string>();
  innerA.set(1, "one");
  innerA.set(2, "two");
  const innerB = new Map<u32, string>();
  innerB.set(10, "ten");
  const outer = new Map<string, Map<u32, string>>();
  outer.set("group-a", innerA);
  outer.set("group-b", innerB);
  const nested = I.nestedRoundtrip(outer);
  assert(nested.size == 2);
  assert(nested.get("group-a").get(1) == "one");
  assert(nested.get("group-a").get(2) == "two");
  assert(nested.get("group-b").get(10) == "ten");

  const single = new Map<u32, string>();
  single.set(1, "one");
  const asMap = I.variantRoundtrip(new I.MapOrString_AsMap(single));
  assert(asMap.tag == 0);
  assert(changetype<I.MapOrString_AsMap>(asMap).value.get(1) == "one");
  const asString = I.variantRoundtrip(new I.MapOrString_AsString("hello"));
  assert(asString.tag == 1);
  assert(changetype<I.MapOrString_AsString>(asString).value == "hello");

  const five = new Map<u32, string>();
  five.set(5, "five");
  const okResult = I.resultRoundtrip(
    new ffi.Result<Map<u32, string>, string>(0, five, ""),
  );
  assert(okResult.isOk() && okResult.okValue.get(5) == "five");
  const errResult = I.resultRoundtrip(
    new ffi.Result<Map<u32, string>, string>(1, new Map<u32, string>(), "bad input"),
  );
  assert(errResult.isErr() && errResult.errValue == "bad input");

  const seven = new Map<u32, string>();
  seven.set(7, "seven");
  const tuple = I.tupleRoundtrip(new ffi.Tuple2<Map<u32, string>, u64>(seven, 42));
  assert(tuple._0.size == 1);
  assert(tuple._0.get(7) == "seven");
  assert(tuple._1 == 42);

  const ninetyNine = new Map<u32, string>();
  ninetyNine.set(99, "ninety-nine");
  const singleOut = I.singleEntryRoundtrip(ninetyNine);
  assert(singleOut.size == 1);
  assert(singleOut.get(99) == "ninety-nine");
}

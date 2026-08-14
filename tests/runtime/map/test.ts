//@ [lang]
//@ path = "stubs/test$maps$to_test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$maps$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function assertBytes(actual: Uint8Array, expected: u8[]): void {
  assert(actual.length == expected.length);
  for (let i = 0; i < actual.length; i++) assert(actual[i] == expected[i]);
}

/// Invert the map, which also proves every entry survived lifting.
function invert(a: Map<u32, string>): Map<string, u32> {
  const out = new Map<string, u32>();
  const keys = a.keys();
  for (let i = 0; i < keys.length; i++) out.set(a.get(keys[i]), keys[i]);
  return out;
}

export function namedRoundtrip(a: Map<u32, string>): Map<string, u32> {
  assert(a.get(1) == "uno");
  assert(a.get(2) == "two");
  return invert(a);
}

export function bytesRoundtrip(a: Map<string, Uint8Array>): Map<string, Uint8Array> {
  assertBytes(a.get("hello"), [0x77, 0x6f, 0x72, 0x6c, 0x64]);
  assertBytes(a.get("bin"), [0, 1, 2]);
  return a;
}

export function emptyRoundtrip(a: Map<u32, string>): Map<u32, string> {
  assert(a.size == 0);
  return a;
}

export function optionRoundtrip(
  a: Map<string, ffi.Option<u32>>,
): Map<string, ffi.Option<u32>> {
  assert(a.get("some").isSome() && a.get("some").value == 42);
  assert(a.get("none").isNone());
  return a;
}

export function recordRoundtrip(a: E.LabeledEntry): E.LabeledEntry {
  assert(a.label == "test-label");
  assert(a.values.size == 2);
  return a;
}

export function inlineRoundtrip(a: Map<u32, string>): Map<string, u32> {
  return invert(a);
}

export function largeRoundtrip(a: Map<u32, string>): Map<u32, string> {
  assert(a.size == 100);
  return a;
}

export function multiParamRoundtrip(
  a: Map<u32, string>,
  b: Map<string, Uint8Array>,
): ffi.Tuple2<Map<string, u32>, Map<string, Uint8Array>> {
  return new ffi.Tuple2<Map<string, u32>, Map<string, Uint8Array>>(invert(a), b);
}

export function nestedRoundtrip(
  a: Map<string, Map<u32, string>>,
): Map<string, Map<u32, string>> {
  return a;
}

export function variantRoundtrip(a: E.MapOrString): E.MapOrString {
  return a;
}

export function resultRoundtrip(
  a: ffi.Result<Map<u32, string>, string>,
): ffi.Result<Map<u32, string>, string> {
  return a;
}

export function tupleRoundtrip(
  a: ffi.Tuple2<Map<u32, string>, u64>,
): ffi.Tuple2<Map<u32, string>, u64> {
  return a;
}

export function singleEntryRoundtrip(a: Map<u32, string>): Map<u32, string> {
  assert(a.size == 1);
  return a;
}

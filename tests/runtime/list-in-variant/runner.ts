//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as I from "../imports/test$list_in_variant$to_test";

function assertEq(actual: string, expected: string): void {
  if (actual != expected) unreachable();
}

function list(items: string[]): Array<string> {
  const out = new Array<string>(items.length);
  for (let i = 0; i < items.length; i++) out[i] = items[i];
  return out;
}

function noList(): Array<string> {
  return new Array<string>(0);
}

export function run(): void {
  // A list inside an option, variant, or result is lowered from within a
  // payload block; its buffer has to outlive the call and then be released.
  assertEq(
    I.listInOption(new ffi.Option<Array<string>>(1, list(["hello", "world"]))),
    "hello,world",
  );
  assertEq(I.listInOption(new ffi.Option<Array<string>>(0, noList())), "none");

  assertEq(
    I.listInVariant(new I.PayloadOrEmpty_WithData(list(["foo", "bar", "baz"]))),
    "foo,bar,baz",
  );
  assertEq(I.listInVariant(new I.PayloadOrEmpty_Empty()), "empty");

  assertEq(
    I.listInResult(new ffi.Result<Array<string>, string>(0, list(["a", "b", "c"]), "")),
    "a,b,c",
  );
  assertEq(
    I.listInResult(new ffi.Result<Array<string>, string>(1, noList(), "oops")),
    "err:oops",
  );

  let summary = I.listInOptionWithReturn(
    new ffi.Option<Array<string>>(1, list(["hello", "world"])),
  );
  if (summary.count != 2) unreachable();
  assertEq(summary.label, "hello,world");

  summary = I.listInOptionWithReturn(new ffi.Option<Array<string>>(0, noList()));
  if (summary.count != 0) unreachable();
  assertEq(summary.label, "none");

  // Contrast case: a list that is itself the top-level argument.
  assertEq(I.topLevelList(list(["x", "y", "z"])), "x,y,z");
}

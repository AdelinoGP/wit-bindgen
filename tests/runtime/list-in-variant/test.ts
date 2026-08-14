//@ [lang]
//@ path = "stubs/test$list_in_variant$to_test.ts"

import * as ffi from "../ffi";
import * as E from "../exports/test$list_in_variant$to_test";

function join(items: Array<string>): string {
  return items.join(",");
}

export function listInOption(data: ffi.Option<Array<string>>): string {
  return data.isSome() ? join(data.value) : "none";
}

export function listInVariant(data: E.PayloadOrEmpty): string {
  if (data.tag == 1) return join(changetype<E.PayloadOrEmpty_WithData>(data).value);
  return "empty";
}

export function listInResult(data: ffi.Result<Array<string>, string>): string {
  return data.isOk() ? join(data.okValue) : "err:" + data.errValue;
}

export function listInOptionWithReturn(data: ffi.Option<Array<string>>): E.Summary {
  if (data.isSome()) return new E.Summary(<u32>data.value.length, join(data.value));
  return new E.Summary(0, "none");
}

export function topLevelList(items: Array<string>): string {
  return join(items);
}

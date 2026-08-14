//@ [lang]
//@ path = "stubs/world.ts"

import * as ffi from "../ffi";
import * as i_test$options$to_test from "../imports/test$options$to_test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function noneU32(): ffi.Option<u32> {
  return new ffi.Option<u32>(0, 0);
}

export function run(): void {
  i_test$options$to_test.optionNoneParam(new ffi.Option<string>(0, ""));
  i_test$options$to_test.optionSomeParam(new ffi.Option<string>(1, "foo"));

  assert(i_test$options$to_test.optionNoneResult().isNone());

  const some = i_test$options$to_test.optionSomeResult();
  assert(some.isSome());
  assert(some.value == "foo");

  const roundtripped = i_test$options$to_test.optionRoundtrip(
    new ffi.Option<string>(1, "foo"),
  );
  assert(roundtripped.isSome());
  assert(roundtripped.value == "foo");

  const someSome = i_test$options$to_test.doubleOptionRoundtrip(
    new ffi.Option<ffi.Option<u32>>(1, new ffi.Option<u32>(1, 42)),
  );
  assert(someSome.isSome());
  assert(someSome.value.isSome());
  assert(someSome.value.value == 42);

  const someNone = i_test$options$to_test.doubleOptionRoundtrip(
    new ffi.Option<ffi.Option<u32>>(1, noneU32()),
  );
  assert(someNone.isSome());
  assert(someNone.value.isNone());

  const none = i_test$options$to_test.doubleOptionRoundtrip(
    new ffi.Option<ffi.Option<u32>>(0, noneU32()),
  );
  assert(none.isNone());
}

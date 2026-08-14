//@ [lang]
//@ path = "stubs/world.ts"

import * as i_test$strings$to_test from "../imports/test$strings$to_test";

function assertEq(actual: string, expected: string): void {
  if (actual != expected) unreachable();
}

export function run(): void {
  // Repeat: post-return and parameter cleanup only show up as corruption or
  // exhaustion after the same buffers have been recycled a few times.
  for (let i = 0; i < 64; i++) {
    i_test$strings$to_test.takeBasic("latin utf16");
    assertEq(i_test$strings$to_test.returnUnicode(), "🚀🚀🚀 𠈄𓀀");
    assertEq(i_test$strings$to_test.returnEmpty(), "");
    assertEq(i_test$strings$to_test.roundtrip("🚀🚀🚀 𠈄𓀀"), "🚀🚀🚀 𠈄𓀀");
  }
}

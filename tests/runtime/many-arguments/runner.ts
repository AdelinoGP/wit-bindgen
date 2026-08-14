//@ [lang]
//@ path = "stubs/world.ts"

import * as i_test$many_arguments$to_test from "../imports/test$many_arguments$to_test";

export function run(): void {
  // 16 u64s exceed the flat-parameter limit, so this goes through the
  // indirect-parameter path in both directions.
  i_test$many_arguments$to_test.manyArguments(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
  );
}

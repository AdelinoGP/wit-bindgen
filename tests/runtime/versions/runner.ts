//@ [lang]
//@ path = "stubs/world.ts"

import * as v1 from "../imports/test$dep$test$0_1_0";
import * as v2 from "../imports/test$dep$test$0_2_0";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  assert(v1.x() == 1.0);
  assert(v1.y(1.0) == 2.0);

  assert(v2.x() == 2.0);
  assert(v2.z(1.0, 1.0) == 4.0);
}

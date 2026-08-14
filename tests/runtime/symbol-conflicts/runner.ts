//@ [lang]
//@ path = "stubs/world.ts"

import * as foo1 from "../imports/my$inline$foo1";
import * as foo2 from "../imports/my$inline$foo2";
import * as bar1 from "../imports/my$inline$bar1";
import * as bar2 from "../imports/my$inline$bar2";

export function run(): void {
  foo1.foo();
  foo2.foo();
  if (bar1.bar() != "") unreachable();
  if (bar2.bar() != "") unreachable();
}

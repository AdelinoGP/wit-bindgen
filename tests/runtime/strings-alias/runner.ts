//@ [lang]
//@ path = "stubs/world.ts"

import * as cat from "../imports/cat";

export function run(): void {
  cat.foo("hello");
  if (cat.bar() != "world") unreachable();
}

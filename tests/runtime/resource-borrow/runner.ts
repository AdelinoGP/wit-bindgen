//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$resource_borrow$to_test";

export function run(): void {
  const thing = I.constructorThing(42);
  // `foo` only borrows, so the handle is still ours to drop afterwards.
  if (I.foo(thing) != 42 + 1 + 2) unreachable();
  thing.drop();
}

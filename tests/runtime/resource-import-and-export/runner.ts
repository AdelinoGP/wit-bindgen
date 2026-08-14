//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$resource_import_and_export$test";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  const thing1 = I.constructorThing(42);

  // 42 + 1 (constructor) + 1 (constructor) + 2 (foo) + 2 (foo)
  assert(I.methodThingFoo(thing1) == 48);

  // 33 + 3 (bar) + 3 (bar) + 2 (foo) + 2 (foo)
  I.methodThingBar(thing1, 33);
  assert(I.methodThingFoo(thing1) == 43);

  const thing2 = I.constructorThing(81);
  const thing3 = I.staticThingBaz(thing1, thing2);
  assert(
    I.methodThingFoo(thing3) ==
      33 + 3 + 3 + 81 + 1 + 1 + 2 + 2 + 4 + 1 + 2 + 4 + 1 + 1 + 2 + 2,
  );
  thing3.drop();
}

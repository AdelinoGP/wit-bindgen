//@ [lang]
//@ path = "stubs/test$resource_import_and_export$test.ts"

import * as I from "../imports/test$resource_import_and_export$test";

// Each exported thing wraps an imported one. Only the raw handle is kept, so
// nothing unmanaged is held through a managed field.
export class Thing {
  constructor(public inner: i32) {}
  __onDrop(): void {}
}

function wrap(handle: i32): I.Thing {
  return new I.Thing(handle);
}

export function constructorThing(v: u32): Thing {
  return new Thing(I.constructorThing(v + 1).handle);
}

export function methodThingFoo(self: Thing): u32 {
  return I.methodThingFoo(wrap(self.inner)) + 2;
}

export function methodThingBar(self: Thing, v: u32): void {
  I.methodThingBar(wrap(self.inner), v + 3);
}

export function staticThingBaz(a: Thing, b: Thing): Thing {
  const combined = I.staticThingBaz(wrap(a.inner), wrap(b.inner));
  const result = I.methodThingFoo(combined) + 4;
  combined.drop();
  return new Thing(I.constructorThing(result + 1).handle);
}

// @@file: stubs/world.ts

import * as I from "../imports/test$resource_import_and_export$test";
import * as world from "../world";

// World-level import and export of the same signature: hand the owned handle
// straight through.
export function toplevelExport(a: I.Thing): I.Thing {
  return world.toplevelImport(a);
}

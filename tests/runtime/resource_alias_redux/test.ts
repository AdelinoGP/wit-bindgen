//@ [lang]
//@ path = "stubs/test$resource_alias_redux$resource_alias1.ts"

import * as A1 from "../exports/test$resource_alias_redux$resource_alias1";

export class Thing {
  constructor(public contents: string) {}
  __onDrop(): void {}
}

export function constructorThing(s: string): Thing {
  return new Thing(s + " GuestThing");
}

export function methodThingGet(self: Thing): string {
  return self.contents + " GuestThing.get";
}

export function a(f: A1.Foo): Array<Thing> {
  const out = new Array<Thing>(1);
  out[0] = f.thing;
  return out;
}

// @@file: stubs/test$resource_alias_redux$resource_alias2.ts

import * as A1 from "../exports/test$resource_alias_redux$resource_alias1";
import * as A2 from "../exports/test$resource_alias_redux$resource_alias2";

// `resource-alias2` uses `resource-alias1`'s resource under two record names;
// both hand back the same class.
export function b(f: A2.Foo, g: A1.Foo): Array<A1.Thing> {
  const out = new Array<A1.Thing>(2);
  out[0] = f.thing;
  out[1] = g.thing;
  return out;
}

// @@file: stubs/the_test.ts

import * as A1 from "../exports/test$resource_alias_redux$resource_alias1";

export function test(things: Array<A1.Thing>): Array<A1.Thing> {
  return things;
}

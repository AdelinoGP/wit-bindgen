//@ [lang]
//@ path = "stubs/test$resource_borrow_in_record$to_test.ts"

import * as E from "../exports/test$resource_borrow_in_record$to_test";

export class Thing {
  constructor(public contents: string) {}
  __onDrop(): void {}
}

export function constructorThing(s: string): Thing {
  return new Thing(s + " new");
}

export function methodThingGet(self: Thing): string {
  return self.contents + " get";
}

// Each `foo` borrows a thing; the results are freshly owned things handed
// back to the caller.
export function test(a: Array<E.Foo>): Array<Thing> {
  const out = new Array<Thing>(a.length);
  for (let i = 0; i < a.length; i++) {
    out[i] = new Thing(a[i].thing.contents + " test");
  }
  return out;
}

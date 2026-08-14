//@ [lang]
//@ path = "stubs/test$resource_with_lists$test.ts"

import * as I from "../imports/test$resource_with_lists$test";

function bytes(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = <u8>s.charCodeAt(i);
  return out;
}

function concat(a: Uint8Array, suffix: string): Uint8Array {
  const tail = bytes(suffix);
  const out = new Uint8Array(a.length + tail.length);
  out.set(a, 0);
  out.set(tail, a.length);
  return out;
}

/// Only the imported handle is kept, so nothing unmanaged is held in a
/// managed field.
export class Thing {
  constructor(public inner: i32) {}
  __onDrop(): void {}
}

function wrap(handle: i32): I.Thing {
  return new I.Thing(handle);
}

export function constructorThing(l: Uint8Array): Thing {
  return new Thing(I.constructorThing(concat(l, " Thing")).handle);
}

export function methodThingFoo(self: Thing): Uint8Array {
  return concat(I.methodThingFoo(wrap(self.inner)), " Thing.foo");
}

export function methodThingBar(self: Thing, l: Uint8Array): void {
  I.methodThingBar(wrap(self.inner), concat(l, " Thing.bar"));
}

export function staticThingBaz(l: Uint8Array): Uint8Array {
  return concat(I.staticThingBaz(concat(l, " Thing.baz")), " Thing.baz again");
}

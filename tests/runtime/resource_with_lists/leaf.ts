//@ [lang]
//@ path = "stubs/test$resource_with_lists$test.ts"

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

export class Thing {
  constructor(public contents: Uint8Array) {}
  __onDrop(): void {}
}

export function constructorThing(l: Uint8Array): Thing {
  return new Thing(concat(l, " HostThing"));
}

export function methodThingFoo(self: Thing): Uint8Array {
  return concat(self.contents, " HostThing.foo");
}

export function methodThingBar(self: Thing, l: Uint8Array): void {
  self.contents = concat(l, " HostThing.bar");
}

export function staticThingBaz(l: Uint8Array): Uint8Array {
  return concat(l, " HostThing.baz");
}

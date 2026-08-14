//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$resource_with_lists$test";

function bytes(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = <u8>s.charCodeAt(i);
  return out;
}

function assertBytes(actual: Uint8Array, expected: string): void {
  const want = bytes(expected);
  if (actual.length != want.length) unreachable();
  for (let i = 0; i < actual.length; i++) {
    if (actual[i] != want[i]) unreachable();
  }
}

export function run(): void {
  const thing = I.constructorThing(bytes("Hi"));

  assertBytes(
    I.methodThingFoo(thing),
    "Hi Thing HostThing HostThing.foo Thing.foo",
  );

  I.methodThingBar(thing, bytes("Hola"));
  assertBytes(
    I.methodThingFoo(thing),
    "Hola Thing.bar HostThing.bar HostThing.foo Thing.foo",
  );

  assertBytes(
    I.staticThingBaz(bytes("Ohayo Gozaimas")),
    "Ohayo Gozaimas Thing.baz HostThing.baz Thing.baz again",
  );

  thing.drop();
}

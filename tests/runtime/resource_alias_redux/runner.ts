//@ [lang]
//@ path = "stubs/world.ts"

import * as A1 from "../imports/test$resource_alias_redux$resource_alias1";
import * as A2 from "../imports/test$resource_alias_redux$resource_alias2";
import * as TheTest from "../imports/the_test";

function assertEq(actual: string, expected: string): void {
  if (actual != expected) unreachable();
}

function list(things: A1.Thing[]): Array<A1.Thing> {
  const out = new Array<A1.Thing>(things.length);
  for (let i = 0; i < things.length; i++) out[i] = things[i];
  return out;
}

export function run(): void {
  // A world-level interface that `use`s the resource.
  let result = TheTest.test(list([A1.constructorThing("Ni Hao")]));
  if (result.length != 1) unreachable();
  assertEq(A1.methodThingGet(result[0]), "Ni Hao GuestThing GuestThing.get");
  result[0].drop();

  // The defining interface.
  result = A1.a(new A1.Foo(A1.constructorThing("Ciao")));
  if (result.length != 1) unreachable();
  assertEq(A1.methodThingGet(result[0]), "Ciao GuestThing GuestThing.get");
  result[0].drop();

  // The aliasing interface, with the resource reached through both its own
  // record and the aliased one.
  result = A2.b(
    new A2.Foo(A1.constructorThing("Ciao")),
    new A1.Foo(A1.constructorThing("Aloha")),
  );
  if (result.length != 2) unreachable();
  assertEq(A1.methodThingGet(result[0]), "Ciao GuestThing GuestThing.get");
  assertEq(A1.methodThingGet(result[1]), "Aloha GuestThing GuestThing.get");
  result[0].drop();
  result[1].drop();
}

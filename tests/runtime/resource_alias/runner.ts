//@ [lang]
//@ path = "stubs/world.ts"

import * as E1 from "../imports/test$resource_alias$e1";
import * as E2 from "../imports/test$resource_alias$e2";

export function run(): void {
  const first = E1.a(new E1.Foo(E1.constructorX(42)));
  if (first.length != 1) unreachable();

  // `e2.a` takes the same resource under an alias, once owned inside each
  // record and once borrowed.
  const borrowed = E1.constructorX(8);
  const second = E2.a(
    new E2.Foo(E1.constructorX(7)),
    new E1.Foo(E1.constructorX(8)),
    borrowed,
  );
  if (second.length != 2) unreachable();
  borrowed.drop();
}

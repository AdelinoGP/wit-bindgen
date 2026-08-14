//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/test$resource_borrow_in_record$to_test";

export function run(): void {
  const first = I.constructorThing("Bonjour");
  const second = I.constructorThing("mon cher");

  const input = new Array<I.Foo>(2);
  input[0] = new I.Foo(first);
  input[1] = new I.Foo(second);

  const result = I.test(input);
  if (result.length != 2) unreachable();
  if (I.methodThingGet(result[0]) != "Bonjour new test get") unreachable();
  if (I.methodThingGet(result[1]) != "mon cher new test get") unreachable();

  result[0].drop();
  result[1].drop();
  first.drop();
  second.drop();
}

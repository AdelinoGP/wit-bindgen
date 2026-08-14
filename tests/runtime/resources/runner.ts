//@ [lang]
//@ path = "stubs/world.ts"

import * as I from "../imports/exports";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  assert(I.testImports().isOk());

  const x = I.constructorX(5);
  assert(I.methodXGetA(x) == 5);
  I.methodXSetA(x, 10);
  assert(I.methodXGetA(x) == 10);

  const z1 = I.constructorZ(10);
  assert(I.methodZGetA(z1) == 10);
  const z2 = I.constructorZ(20);
  assert(I.methodZGetA(z2) == 20);

  // `add` consumes the owned handle and hands one back.
  const xadd = I.staticXAdd(x, 5);
  assert(I.methodXGetA(xadd) == 15);

  const zadd = I.add(z1, z2);
  assert(I.methodZGetA(zadd) == 30);

  const droppedStart = I.staticZNumDropped();

  z1.drop();
  z2.drop();

  I.consume(xadd);

  const droppedEnd = I.staticZNumDropped();
  if (droppedStart != 0) {
    assert(droppedEnd == droppedStart + 2);
  }

  const kebab = I.constructorKebabCase(7);
  assert(I.methodKebabCaseGetA(kebab) == 7);
  assert(I.staticKebabCaseTakeOwned(kebab) == 7);

  zadd.drop();
}

//@ [lang]
//@ path = "stubs/exports.ts"

import * as ffi from "../ffi";
import * as I from "../imports/imports";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export class X {
  constructor(public value: i32) {}
  __onDrop(): void {}
}

/// Counts destructor calls so the runner can observe that dropping an owned
/// handle really does run `__onDrop`.
let droppedZs: u32 = 0;

export class Z {
  constructor(public value: i32) {}
  __onDrop(): void {
    droppedZs++;
  }
}

export class KebabCase {
  constructor(public value: u32) {}
  __onDrop(): void {}
}

export function constructorX(a: i32): X {
  return new X(a);
}

export function methodXGetA(self: X): i32 {
  return self.value;
}

export function methodXSetA(self: X, a: i32): void {
  self.value = a;
}

/// Takes an owned handle and hands the same instance straight back.
export function staticXAdd(x: X, a: i32): X {
  x.value += a;
  return x;
}

export function constructorZ(a: i32): Z {
  return new Z(a);
}

export function methodZGetA(self: Z): i32 {
  return self.value;
}

export function staticZNumDropped(): u32 {
  return droppedZs + 1;
}

export function add(a: Z, b: Z): Z {
  return new Z(a.value + b.value);
}

export function consume(x: X): void {}

export function constructorKebabCase(a: u32): KebabCase {
  return new KebabCase(a);
}

export function methodKebabCaseGetA(self: KebabCase): u32 {
  return self.value;
}

export function staticKebabCaseTakeOwned(k: KebabCase): u32 {
  return k.value;
}

export function testImports(): ffi.Result<i32, string> {
  const y = I.constructorY(10);
  assert(I.methodYGetA(y) == 10);
  I.methodYSetA(y, 20);
  assert(I.methodYGetA(y) == 20);
  const y2 = I.staticYAdd(y, 20);
  assert(I.methodYGetA(y2) == 40);
  y2.drop();

  // Several live instances at once, to prove the handles stay distinct.
  const a = I.constructorY(1);
  const b = I.constructorY(2);
  assert(I.methodYGetA(a) == 1);
  assert(I.methodYGetA(b) == 2);
  I.methodYSetA(a, 10);
  I.methodYSetA(b, 20);
  assert(I.methodYGetA(a) == 10);
  assert(I.methodYGetA(b) == 20);
  const a2 = I.staticYAdd(a, 20);
  const b2 = I.staticYAdd(b, 30);
  assert(I.methodYGetA(a2) == 30);
  assert(I.methodYGetA(b2) == 50);
  a2.drop();
  b2.drop();

  return new ffi.Result<i32, string>(0, 0, "");
}

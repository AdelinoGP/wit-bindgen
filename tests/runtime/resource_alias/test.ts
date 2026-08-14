//@ [lang]
//@ path = "stubs/test$resource_alias$e1.ts"

import * as E1 from "../exports/test$resource_alias$e1";

export class X {
  constructor(public value: u32) {}
  __onDrop(): void {}
}

export function constructorX(v: u32): X {
  return new X(v);
}

export function a(f: E1.Foo): Array<X> {
  const out = new Array<X>(1);
  out[0] = f.x;
  return out;
}

// @@file: stubs/test$resource_alias$e2.ts

import * as E1 from "../exports/test$resource_alias$e1";
import * as E2 from "../exports/test$resource_alias$e2";

// `e2` aliases `e1`'s resource, so both records hand back the same class.
export function a(f: E2.Foo, g: E1.Foo, h: E1.X): Array<E1.X> {
  const out = new Array<E1.X>(2);
  out[0] = f.x;
  out[1] = g.x;
  return out;
}

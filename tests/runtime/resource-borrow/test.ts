//@ [lang]
//@ path = "stubs/test$resource_borrow$to_test.ts"

export class Thing {
  constructor(public val: u32) {}
  __onDrop(): void {}
}

export function thing(v: u32): Thing {
  return new Thing(v + 1);
}

export function foo(v: Thing): u32 {
  return v.val + 2;
}

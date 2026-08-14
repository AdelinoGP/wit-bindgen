//@ [lang]
//@ path = "stubs/test$resource_import_and_export$test.ts"

export class Thing {
  constructor(public value: u32) {}
  __onDrop(): void {}
}

export function thing(v: u32): Thing {
  return new Thing(v + 1);
}

export function foo(self: Thing): u32 {
  return self.value + 2;
}

export function bar(self: Thing, v: u32): void {
  self.value = v + 3;
}

export function baz(a: Thing, b: Thing): Thing {
  return new Thing(foo(a) + foo(b) + 4 + 1);
}

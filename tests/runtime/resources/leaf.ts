//@ [lang]
//@ path = "stubs/imports.ts"

export class Y {
  constructor(public value: i32) {}
  __onDrop(): void {}
}

export function constructorY(a: i32): Y {
  return new Y(a);
}

export function methodYGetA(self: Y): i32 {
  return self.value;
}

export function methodYSetA(self: Y, a: i32): void {
  self.value = a;
}

export function staticYAdd(y: Y, a: i32): Y {
  return new Y(y.value + a);
}

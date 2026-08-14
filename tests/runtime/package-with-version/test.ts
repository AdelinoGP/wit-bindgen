//@ [lang]
//@ path = "stubs/my$inline$foo$0_0_0.ts"

export class Bar {
  constructor() {}
  __onDrop(): void {}
}

export function bar(): Bar {
  return new Bar();
}

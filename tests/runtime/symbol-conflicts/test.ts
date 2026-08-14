//@ [lang]
//@ path = "stubs/my$inline$foo1.ts"

export function foo(): void {}

// @@file: stubs/my$inline$foo2.ts

// Same function name in a second interface: the generated glue has to keep
// the two apart.
export function foo(): void {}

// @@file: stubs/my$inline$bar1.ts

export function bar(): string {
  return "";
}

// @@file: stubs/my$inline$bar2.ts

export function bar(): string {
  return "";
}

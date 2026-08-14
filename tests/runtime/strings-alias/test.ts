//@ [lang]
//@ path = "stubs/cat.ts"

export function foo(x: string): void {
  if (x != "hello") unreachable();
}

export function bar(): string {
  return "world";
}

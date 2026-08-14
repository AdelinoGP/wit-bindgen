//@ [lang]
//@ path = "stubs/cat.ts"

function assertBytes(actual: Uint8Array, expected: string): void {
  if (actual.length != expected.length) unreachable();
  for (let i = 0; i < actual.length; i++) {
    if (actual[i] != <u8>expected.charCodeAt(i)) unreachable();
  }
}

export function foo(x: Uint8Array): void {
  assertBytes(x, "hello");
}

export function bar(): Uint8Array {
  const out = new Uint8Array(5);
  const word = "world";
  for (let i = 0; i < 5; i++) out[i] = <u8>word.charCodeAt(i);
  return out;
}

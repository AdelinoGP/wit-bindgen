//@ [lang]
//@ path = "stubs/world.ts"

import * as cat from "../imports/cat";

function bytes(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = <u8>s.charCodeAt(i);
  return out;
}

export function run(): void {
  cat.foo(bytes("hello"));

  const got = cat.bar();
  const want = bytes("world");
  if (got.length != want.length) unreachable();
  for (let i = 0; i < got.length; i++) {
    if (got[i] != want[i]) unreachable();
  }
}

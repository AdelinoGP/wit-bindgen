//@ [lang]
//@ path = "stubs/test$strings$to_test.ts"

function assertEq(actual: string, expected: string): void {
  if (actual != expected) unreachable();
}

export function takeBasic(s: string): void {
  assertEq(s, "latin utf16");
}

export function returnUnicode(): string {
  return "🚀🚀🚀 𠈄𓀀";
}

export function returnEmpty(): string {
  return "";
}

export function roundtrip(s: string): string {
  return s;
}

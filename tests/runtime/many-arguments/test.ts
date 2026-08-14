//@ [lang]
//@ path = "stubs/test$many_arguments$to_test.ts"

function assertEq(actual: u64, expected: u64): void {
  if (actual != expected) unreachable();
}

export function manyArguments(
  a1: u64, a2: u64, a3: u64, a4: u64,
  a5: u64, a6: u64, a7: u64, a8: u64,
  a9: u64, a10: u64, a11: u64, a12: u64,
  a13: u64, a14: u64, a15: u64, a16: u64,
): void {
  assertEq(a1, 1);
  assertEq(a2, 2);
  assertEq(a3, 3);
  assertEq(a4, 4);
  assertEq(a5, 5);
  assertEq(a6, 6);
  assertEq(a7, 7);
  assertEq(a8, 8);
  assertEq(a9, 9);
  assertEq(a10, 10);
  assertEq(a11, 11);
  assertEq(a12, 12);
  assertEq(a13, 13);
  assertEq(a14, 14);
  assertEq(a15, 15);
  assertEq(a16, 16);
}

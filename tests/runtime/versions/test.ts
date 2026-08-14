//@ [lang]
//@ path = "stubs/test$dep$test$0_1_0.ts"

export function x(): f32 {
  return 1.0;
}

export function y(a: f32): f32 {
  return 1.0 + a;
}

// @@file: stubs/test$dep$test$0_2_0.ts

// Same interface name at a different version: the two must stay distinct all
// the way down to the wasm export names.
export function x(): f32 {
  return 2.0;
}

export function z(a: f32, b: f32): f32 {
  return 2.0 + a + b;
}

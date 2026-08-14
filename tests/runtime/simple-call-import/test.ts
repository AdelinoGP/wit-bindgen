//@ [lang]
//@ path = "stubs/a$b$i.ts"

import * as e_a$b$i from "../exports/a$b$i";

@unmanaged
class F extends e_a$b$i.FTask {
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    // Completes inside the initial call, so the import never becomes a
    // waitable for the caller.
    return this.finish();
  }
}

export function f(): e_a$b$i.FTask {
  return new F();
}

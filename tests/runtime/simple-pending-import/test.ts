//@ [lang]
//@ path = "stubs/a$b$i.ts"

import * as async_ from "../async";
import * as e_a$b$i from "../exports/a$b$i";

@unmanaged
class F extends e_a$b$i.FTask {
  private yields: i32 = 0;

  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    // Ten round trips through the generated `[callback]` export before the
    // task returns.
    if (this.yields < 10) {
      this.yields++;
      return async_.Scheduler.yield();
    }
    return this.finish();
  }
}

export function f(): e_a$b$i.FTask {
  return new F();
}

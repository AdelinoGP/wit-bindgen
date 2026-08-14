//@ [lang]
//@ path = "stubs/a$b$i.ts"

import * as async_ from "../async";
import * as e_a$b$i from "../exports/a$b$i";

function assertEq<T>(actual: T, expected: T): void {
  if (actual != expected) unreachable();
}

@unmanaged
class YieldTask extends e_a$b$i.FTask {
  private state: i32 = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.state == 0) {
      // First canonical event: the initial NONE resumption, mirroring
      // exports_test_f_callback in test.c.
      assertEq(event, async_.EVENT_NONE);
      assertEq(waitable, 0);
      assertEq(code, 0);
      this.state = 1;
      async_.threadYield();
      return async_.Scheduler.yield();
    }
    if (this.state == 1) {
      // Resumed after yielding: no waitable is pending, so the host re-enters
      // with another NONE event; the function finishes on this pass.
      assertEq(event, async_.EVENT_NONE);
      assertEq(waitable, 0);
      assertEq(code, 0);
      return this.finish();
    }
    unreachable();
    return async_.CALLBACK_CODE_EXIT;
  }
}

export function f(): e_a$b$i.FTask {
  return new YieldTask();
}

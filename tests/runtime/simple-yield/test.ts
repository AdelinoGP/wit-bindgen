//@ [lang]
//@ path = "exports/a$b$i.ts"

import * as async_ from "../async";

function assertEq<T>(actual: T, expected: T): void {
  if (actual != expected) unreachable();
}

@unmanaged
class YieldTask extends FTask {
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

export function f(): FTask {
  return new YieldTask();
}

export function __exp_0_f(): i32 {
  const status = async_.Scheduler.start(f());
  __finish___exp_0_f(status);
  return status;
}

@external("[export]a:b/i", "[task-return]f")
declare function __task_return_0_f(): void;

@unmanaged
export abstract class FTask extends async_.AsyncTask {
  finished: bool = false;

  finish(): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@inline(false)
export function __finish___exp_0_f(status: i32): void {
  const task = async_.contextGet();
  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;
  if (load<bool>(task + offsetof<FTask>("finished"))) __task_return_0_f();
  async_.Scheduler.release(task);
  async_.Scheduler.complete(task);
}

export function __callback_0_f(event: i32, waitable: i32, code: i32): i32 {
  const status = async_.Scheduler.resume(event, waitable, code);
  __finish___exp_0_f(status);
  return status;
}

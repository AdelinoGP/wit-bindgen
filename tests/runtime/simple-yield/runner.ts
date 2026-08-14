//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "stubs/world.ts"

import * as async_ from "../async";
import * as world from "../world";
import * as i_a$b$i from "../imports/a$b$i";

function assertEq<T>(actual: T, expected: T): void {
  if (actual != expected) unreachable();
}

@unmanaged
class RunTask extends world.RunTask {
  private iteration: i32 = 0;
  private set: i32 = 0;
  // Raw pointer to the in-flight `@unmanaged` subtask; nothing managed may
  // survive a yield.
  private pending: usize = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.pending != 0) {
      const subtask = changetype<i_a$b$i.FSubtask>(this.pending);
      assertEq(event, async_.EVENT_SUBTASK);
      assertEq(waitable, subtask.handle);
      assertEq(async_.subtaskState(code), async_.STATUS_RETURNED);
      this.pending = 0;
      subtask.finish(code);
    }

    while (this.iteration < 32) {
      this.iteration++;
      const subtask = i_a$b$i.f();
      if (subtask.handle == 0) {
        // Completed without ever becoming a waitable.
        assertEq(subtask.state, async_.STATUS_RETURNED);
        subtask.finish(subtask.status);
        continue;
      }
      assertEq(subtask.state, async_.STATUS_STARTED);
      if (this.set == 0) this.set = async_.waitableSetNew();
      async_.waitableJoin(subtask.handle, this.set);
      this.pending = changetype<usize>(subtask);
      return async_.callbackWait(this.set);
    }

    if (this.set != 0) {
      async_.waitableSetDrop(this.set);
      this.set = 0;
    }
    return this.finish();
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

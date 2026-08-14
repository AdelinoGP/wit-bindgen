//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "stubs/world.ts"

import * as async_ from "../async";
import * as world from "../world";
import * as I from "../imports/a$b$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class RunTask extends world.RunTask {
  private set: i32 = 0;
  private pending: usize = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.pending != 0) {
      const subtask = changetype<I.FSubtask>(this.pending);
      assert(event == async_.EVENT_SUBTASK && waitable == subtask.handle);
      assert(async_.subtaskState(code) == async_.STATUS_RETURNED);
      this.pending = 0;
      subtask.finish(code);
      async_.waitableSetDrop(this.set);
      this.set = 0;
      return this.finish();
    }

    assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
    const subtask = I.f();
    if (subtask.handle == 0) {
      subtask.finish(subtask.status);
      return this.finish();
    }
    this.set = async_.waitableSetNew();
    async_.waitableJoin(subtask.handle, this.set);
    this.pending = changetype<usize>(subtask);
    return async_.callbackWait(this.set);
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

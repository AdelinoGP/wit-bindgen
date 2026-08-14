//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "stubs/world.ts"

import * as async_ from "../async";
import * as world from "../world";
import * as i_my$test$i from "../imports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class RunTask extends world.RunTask {
  private iteration: i32 = 0;
  private set: i32 = 0;
  private pending: usize = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.pending != 0) {
      const subtask = changetype<i_my$test$i.ReturnStringSubtask>(this.pending);
      assert(event == async_.EVENT_SUBTASK);
      assert(waitable == subtask.handle);
      this.pending = 0;
      assert(subtask.finish(code) == "hello");
    }

    // Lifting a managed result allocates, so repeat the call: this exercises
    // both the synchronously-completing and the suspended subtask paths, and
    // would surface allocator corruption on a later iteration.
    while (this.iteration < 64) {
      this.iteration++;
      const subtask = i_my$test$i.returnString();
      if (subtask.handle == 0) {
        assert(subtask.finish(subtask.status) == "hello");
        continue;
      }
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

//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "stubs/world.ts"

import * as async_ from "../async";
import * as world from "../world";
import * as i_a$b$i from "../imports/a$b$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class RunTask extends world.RunTask {
  private step: i32 = 0;
  private set: i32 = 0;
  // Raw pointer to the in-flight `@unmanaged` subtask and the status code most
  // recently observed for it. Nothing managed may survive a suspension.
  private subtask: usize = 0;
  private code: i32 = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (event == async_.EVENT_SUBTASK) {
      assert(waitable == this.pendingHandle());
      this.code = code;
    }

    while (true) {
      switch (this.step) {
        case 0: {
          assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
          this.set = async_.waitableSetNew();
          const wait = this.begin(changetype<usize>(i_a$b$i.oneArgument(1)), 1);
          if (wait != -1) return wait;
          continue;
        }

        case 1: {
          changetype<i_a$b$i.OneArgumentSubtask>(this.subtask).finish(this.code);
          const wait = this.begin(changetype<usize>(i_a$b$i.oneResult()), 2);
          if (wait != -1) return wait;
          continue;
        }

        case 2: {
          assert(changetype<i_a$b$i.OneResultSubtask>(this.subtask).finish(this.code) == 2);
          const wait = this.begin(changetype<usize>(i_a$b$i.oneArgumentAndResult(3)), 3);
          if (wait != -1) return wait;
          continue;
        }

        case 3: {
          assert(
            changetype<i_a$b$i.OneArgumentAndResultSubtask>(this.subtask).finish(this.code) == 4
          );
          const wait = this.begin(changetype<usize>(i_a$b$i.twoArguments(5, 6)), 4);
          if (wait != -1) return wait;
          continue;
        }

        case 4: {
          changetype<i_a$b$i.TwoArgumentsSubtask>(this.subtask).finish(this.code);
          const wait = this.begin(changetype<usize>(i_a$b$i.twoArgumentsAndResult(7, 8)), 5);
          if (wait != -1) return wait;
          continue;
        }

        default: {
          assert(
            changetype<i_a$b$i.TwoArgumentsAndResultSubtask>(this.subtask).finish(this.code) == 9
          );
          this.subtask = 0;
          async_.waitableSetDrop(this.set);
          this.set = 0;
          return this.finish();
        }
      }
    }
  }

  /// Every generated `*Subtask` starts with the same scalar header, so one
  /// accessor covers whichever call is in flight.
  private pendingHandle(): i32 {
    return changetype<i_a$b$i.OneArgumentSubtask>(this.subtask).handle;
  }

  /// Record `ptr` as the in-flight subtask, advance to `next`, and return the
  /// callback code to suspend on — or -1 if it already completed.
  private begin(ptr: usize, next: i32): i32 {
    this.subtask = ptr;
    this.step = next;
    const subtask = changetype<i_a$b$i.OneArgumentSubtask>(ptr);
    if (subtask.handle == 0) {
      this.code = subtask.status;
      return -1;
    }
    async_.waitableJoin(subtask.handle, this.set);
    return async_.callbackWait(this.set);
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

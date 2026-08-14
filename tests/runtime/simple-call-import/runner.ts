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
  resume(event: i32, waitable: i32, code: i32): i32 {
    assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);

    // The callee completes inside the initial call, so the subtask returns
    // without ever becoming a waitable — the same property `runner.c` pins.
    const subtask = I.f();
    assert(subtask.state == async_.STATUS_RETURNED);
    assert(subtask.handle == 0);
    subtask.finish(subtask.status);

    return this.finish();
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

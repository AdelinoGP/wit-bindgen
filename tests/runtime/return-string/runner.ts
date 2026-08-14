//@ args = '--async=-run'
//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "world.ts"

import * as async_ from "./async";
import * as i_my$test$i from "./imports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

export function run(): void {
  // Lifting a managed result allocates, so repeat the call: this exercises both
  // the synchronously-completing and the suspended subtask paths, and would
  // surface allocator corruption on a later iteration.
  for (let i = 0; i < 64; i++) {
    const subtask = i_my$test$i.returnString();
    let value: string;
    if (subtask.handle != 0) {
      const set = async_.waitableSetNew();
      async_.waitableJoin(subtask.handle, set);
      const payload = changetype<usize>(memory.data(8));
      assert(async_.waitableSetWait(set, payload) == async_.EVENT_SUBTASK);
      const code = load<i32>(payload + 4);
      value = subtask.finish(code);
      async_.waitableSetDrop(set);
    } else {
      value = subtask.finish(subtask.status);
    }
    assert(value == "hello");
  }
}

export function __exp_18446744073709551615_run(): void {
  run();
}

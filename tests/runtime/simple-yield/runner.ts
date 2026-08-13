//@ args = '--async=-run'
//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "world.ts"

import * as async_ from "./async";
import * as i_a$b$i from "./imports/a$b$i";

function assertEq<T>(actual: T, expected: T): void {
  if (actual != expected) unreachable();
}

export function run(): void {
  for (let i = 0; i < 32; i++) {
    const subtask = i_a$b$i.f();
    assertEq(subtask.state, async_.STATUS_STARTED);
    const task = subtask.handle;
    assert(task != 0);
    const set = async_.waitableSetNew();
    async_.waitableJoin(task, set);
    const payload = changetype<usize>(memory.data(8));
    const event = async_.waitableSetWait(set, payload);
    assertEq(event, async_.EVENT_SUBTASK);
    assertEq(load<i32>(payload), task);
    const status = load<i32>(payload + 4);
    assertEq(status, async_.STATUS_RETURNED);
    subtask.finish(status);
    async_.waitableSetDrop(set);
  }
}

export function __exp_18446744073709551615_run(): void {
  run();
}

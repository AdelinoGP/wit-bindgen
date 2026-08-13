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
    const status = i_a$b$i.f();
    assertEq(async_.subtaskState(status), async_.STATUS_STARTED);
    const task = async_.subtaskHandle(status);
    assert(task != 0);
    const set = async_.waitableSetNew();
    async_.waitableJoin(task, set);
    const payload = changetype<usize>(memory.data(8));
    const event = async_.waitableSetWait(set, payload);
    assertEq(event, async_.EVENT_SUBTASK);
    assertEq(load<i32>(payload), task);
    assertEq(load<i32>(payload + 4), async_.STATUS_RETURNED);
    async_.waitableJoin(task, 0);
    async_.subtaskDrop(task);
    async_.waitableSetDrop(set);
  }
}

export function __exp_18446744073709551615_run(): void {
  run();
}

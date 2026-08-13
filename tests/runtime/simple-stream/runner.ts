//@ args = '--async=-run'
//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "world.ts"

import * as async_ from "./async";
import * as i_my$test$i from "./imports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function reader(pair: i64): i32 {
  return <i32>pair;
}

function writer(pair: i64): i32 {
  return <i32>(pair >> 32);
}

export function run(): void {
  const pair = i_my$test$i.rawImportReadStreamStream0New();
  const read = reader(pair);
  const write = writer(pair);

  assert(i_my$test$i.rawImportReadStreamStream0Write(write, 0, 1) == -1);

  const subtask = i_my$test$i.readStream(read);
  assert(subtask.state == async_.STATUS_STARTED);
  const task = subtask.handle;
  assert(task != 0);

  const set = async_.waitableSetNew();
  const payload = changetype<usize>(memory.data(8));
  async_.waitableJoin(write, set);

  let event = async_.waitableSetWait(set, payload);
  assert(event == async_.EVENT_STREAM_WRITE);
  assert(load<i32>(payload) == write);
  assert(async_.waitableState(load<i32>(payload + 4)) == async_.WAITABLE_COMPLETED);
  assert(async_.waitableCount(load<i32>(payload + 4)) == 1);

  let writeStatus = i_my$test$i.rawImportReadStreamStream0Write(write, 0, 2);
  assert(async_.waitableState(writeStatus) == async_.WAITABLE_COMPLETED);
  assert(async_.waitableCount(writeStatus) == 2);

  writeStatus = i_my$test$i.rawImportReadStreamStream0Write(write, 0, 2);
  assert(writeStatus == -1);
  event = async_.waitableSetWait(set, payload);
  assert(event == async_.EVENT_STREAM_WRITE);
  assert(load<i32>(payload) == write);
  assert(async_.waitableState(load<i32>(payload + 4)) == async_.WAITABLE_DROPPED);
  assert(async_.waitableCount(load<i32>(payload + 4)) == 0);

  async_.waitableJoin(write, 0);
  i_my$test$i.rawImportReadStreamStream0DropWritable(write);

  async_.waitableJoin(task, set);
  event = async_.waitableSetWait(set, payload);
  assert(event == async_.EVENT_SUBTASK);
  assert(load<i32>(payload) == task);
  const status = load<i32>(payload + 4);
  assert(async_.subtaskState(status) == async_.STATUS_RETURNED);
  subtask.finish(status);
  async_.waitableSetDrop(set);
}

export function __exp_18446744073709551615_run(): void {
  run();
}

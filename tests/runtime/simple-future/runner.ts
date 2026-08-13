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

function waitForWrite(handle: i32, expected: i32): void {
  const set = async_.waitableSetNew();
  async_.waitableJoin(handle, set);
  const payload = changetype<usize>(memory.data(8));
  const event = async_.waitableSetWait(set, payload);
  assert(event == async_.EVENT_FUTURE_WRITE);
  assert(load<i32>(payload) == handle);
  assert(async_.waitableState(load<i32>(payload + 4)) == expected);
  assert(async_.waitableCount(load<i32>(payload + 4)) == 0);
  async_.waitableJoin(handle, 0);
  async_.waitableSetDrop(set);
}

export function run(): void {
  {
    const pair = i_my$test$i.rawImportReadFutureFuture0New();
    const read = reader(pair);
    const write = writer(pair);
    assert(i_my$test$i.rawImportReadFutureFuture0Write(write, 0) == -1);
    const status = i_my$test$i.readFuture(read);
    assert(async_.subtaskState(status) == async_.STATUS_RETURNED);
    waitForWrite(write, async_.WAITABLE_COMPLETED);
    i_my$test$i.rawImportReadFutureFuture0DropWritable(write);
  }

  {
    const pair = i_my$test$i.rawImportDropFutureFuture0New();
    const read = reader(pair);
    const write = writer(pair);
    assert(i_my$test$i.rawImportDropFutureFuture0Write(write, 0) == -1);
    const status = i_my$test$i.dropFuture(read);
    assert(async_.subtaskState(status) == async_.STATUS_RETURNED);
    waitForWrite(write, async_.WAITABLE_DROPPED);
    i_my$test$i.rawImportDropFutureFuture0DropWritable(write);
  }
}

export function __exp_18446744073709551615_run(): void {
  run();
}

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

function assertDropped(handle: i32): void {
  const status = i_my$test$i.rawImportPendingImportFuture0Write(handle, 0);
  assert(async_.waitableState(status) == async_.WAITABLE_DROPPED);
  assert(async_.waitableCount(status) == 0);
  i_my$test$i.rawImportPendingImportFuture0DropWritable(handle);
}

export function run(): void {
  const first = i_my$test$i.rawImportPendingImportFuture0New();
  const started = i_my$test$i.pendingImport(reader(first));
  assert(started.state == async_.STATUS_STARTED);
  assert(started.handle != 0);
  const startedStatus = started.cancel();
  assert(async_.subtaskState(startedStatus) == async_.STATUS_RETURNED_CANCELLED);
  assert(async_.subtaskHandle(startedStatus) == 0);
  assert(started.dispose(startedStatus));
  assertDropped(writer(first));

  const second = i_my$test$i.rawImportPendingImportFuture0New();
  i_my$test$i.backpressureSet(true);
  const starting = i_my$test$i.pendingImport(reader(second));
  assert(starting.state == async_.STATUS_STARTING);
  assert(starting.handle != 0);
  const startingStatus = starting.cancel();
  assert(async_.subtaskState(startingStatus) == async_.STATUS_STARTED_CANCELLED);
  assert(async_.subtaskHandle(startingStatus) == 0);
  assert(starting.dispose(startingStatus));
  assertDropped(writer(second));
  i_my$test$i.backpressureSet(false);
}

export function __exp_18446744073709551615_run(): void {
  run();
}

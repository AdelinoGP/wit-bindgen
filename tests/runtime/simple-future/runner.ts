//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "stubs/world.ts"

import * as async_ from "../async";
import * as world from "../world";
import * as i_my$test$i from "../imports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

function reader(pair: i64): i32 {
  return <i32>pair;
}

function writer(pair: i64): i32 {
  return <i32>(pair >> 32);
}

@unmanaged
class RunTask extends world.RunTask {
  private step: i32 = 0;
  private set: i32 = 0;
  private write: i32 = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    switch (this.step) {
      case 0: {
        assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
        const pair = i_my$test$i.rawImportReadFutureFuture0New();
        this.write = writer(pair);
        assert(i_my$test$i.rawImportReadFutureFuture0Write(this.write, 0) == -1);
        const subtask = i_my$test$i.readFuture(reader(pair));
        assert(subtask.state == async_.STATUS_RETURNED);
        subtask.finish(subtask.status);
        this.set = async_.waitableSetNew();
        async_.waitableJoin(this.write, this.set);
        this.step = 1;
        return async_.callbackWait(this.set);
      }
      case 1: {
        this.expectWrite(event, waitable, code, async_.WAITABLE_COMPLETED);
        const pair = i_my$test$i.rawImportDropFutureFuture0New();
        this.write = writer(pair);
        assert(i_my$test$i.rawImportDropFutureFuture0Write(this.write, 0) == -1);
        const subtask = i_my$test$i.dropFuture(reader(pair));
        assert(subtask.state == async_.STATUS_RETURNED);
        subtask.finish(subtask.status);
        async_.waitableJoin(this.write, this.set);
        this.step = 2;
        return async_.callbackWait(this.set);
      }
      default: {
        this.expectWrite(event, waitable, code, async_.WAITABLE_DROPPED);
        async_.waitableSetDrop(this.set);
        this.set = 0;
        return this.finish();
      }
    }
  }

  /// Consume the completion of the pending write and release its handle.
  private expectWrite(event: i32, waitable: i32, code: i32, expected: i32): void {
    assert(event == async_.EVENT_FUTURE_WRITE);
    assert(waitable == this.write);
    assert(async_.waitableState(code) == expected);
    assert(async_.waitableCount(code) == 0);
    async_.waitableJoin(this.write, 0);
    if (expected == async_.WAITABLE_COMPLETED) {
      i_my$test$i.rawImportReadFutureFuture0DropWritable(this.write);
    } else {
      i_my$test$i.rawImportDropFutureFuture0DropWritable(this.write);
    }
    this.write = 0;
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

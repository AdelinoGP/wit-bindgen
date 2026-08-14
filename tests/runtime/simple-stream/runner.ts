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
  private subtask: usize = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    switch (this.step) {
      case 0: {
        assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
        const pair = i_my$test$i.rawImportReadStreamStream0New();
        this.write = writer(pair);
        assert(i_my$test$i.rawImportReadStreamStream0Write(this.write, 0, 1) == -1);

        const subtask = i_my$test$i.readStream(reader(pair));
        assert(subtask.state == async_.STATUS_STARTED);
        assert(subtask.handle != 0);
        this.subtask = changetype<usize>(subtask);

        this.set = async_.waitableSetNew();
        async_.waitableJoin(this.write, this.set);
        this.step = 1;
        return async_.callbackWait(this.set);
      }
      case 1: {
        assert(event == async_.EVENT_STREAM_WRITE && waitable == this.write);
        assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
        assert(async_.waitableCount(code) == 1);

        let status = i_my$test$i.rawImportReadStreamStream0Write(this.write, 0, 2);
        assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
        assert(async_.waitableCount(status) == 2);

        status = i_my$test$i.rawImportReadStreamStream0Write(this.write, 0, 2);
        assert(status == -1);
        this.step = 2;
        return async_.callbackWait(this.set);
      }
      case 2: {
        assert(event == async_.EVENT_STREAM_WRITE && waitable == this.write);
        assert(async_.waitableState(code) == async_.WAITABLE_DROPPED);
        assert(async_.waitableCount(code) == 0);

        async_.waitableJoin(this.write, 0);
        i_my$test$i.rawImportReadStreamStream0DropWritable(this.write);
        this.write = 0;

        const subtask = changetype<i_my$test$i.ReadStreamSubtask>(this.subtask);
        async_.waitableJoin(subtask.handle, this.set);
        this.step = 3;
        return async_.callbackWait(this.set);
      }
      default: {
        const subtask = changetype<i_my$test$i.ReadStreamSubtask>(this.subtask);
        assert(event == async_.EVENT_SUBTASK && waitable == subtask.handle);
        assert(async_.subtaskState(code) == async_.STATUS_RETURNED);
        this.subtask = 0;
        subtask.finish(code);
        async_.waitableSetDrop(this.set);
        this.set = 0;
        return this.finish();
      }
    }
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

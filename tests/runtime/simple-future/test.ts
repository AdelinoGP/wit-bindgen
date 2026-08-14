//@ [lang]
//@ path = "stubs/my$test$i.ts"

import * as async_ from "../async";
import * as e_my$test$i from "../exports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class ReadFuture extends e_my$test$i.ReadFutureTask {
  private reader: i32;
  private set: i32 = 0;
  private state: i32 = 0;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.state == 0) {
      assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
      this.state = 1;
      const status = this.startFuture0Read(this.reader);
      if (status == -1) {
        this.set = async_.waitableSetNew();
        async_.waitableJoin(this.reader, this.set);
        return async_.callbackWait(this.set);
      }
      assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
    } else {
      assert(event == async_.EVENT_FUTURE_READ && waitable == this.reader);
      assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
      async_.waitableJoin(this.reader, 0);
    }
    this.finishFuture0Read();
    e_my$test$i.rawExportReadFutureFuture0DropReadable(this.reader);
    if (this.set != 0) async_.waitableSetDrop(this.set);
    return this.finish();
  }
}

export function readFuture(x: i32): e_my$test$i.ReadFutureTask {
  return new ReadFuture(x);
}

@unmanaged
class DropFuture extends e_my$test$i.DropFutureTask {
  private reader: i32;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
    e_my$test$i.rawExportDropFutureFuture0DropReadable(this.reader);
    return this.finish();
  }
}

export function dropFuture(x: i32): e_my$test$i.DropFutureTask {
  return new DropFuture(x);
}

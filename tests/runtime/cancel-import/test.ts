//@ [lang]
//@ path = "stubs/my$test$i.ts"

import * as async_ from "../async";
import * as e_my$test$i from "../exports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class PendingImport extends e_my$test$i.PendingImportTask {
  private reader: i32;
  private set: i32 = 0;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (event == async_.EVENT_NONE) {
      assert(waitable == 0 && code == 0);
      // Generated typed endpoint helper: blocks, so the read stays pending.
      assert(this.startFuture0Read(this.reader) == -1);
      this.set = async_.waitableSetNew();
      async_.waitableJoin(this.reader, this.set);
      return async_.callbackWait(this.set);
    }

    if (event == async_.EVENT_CANCEL) {
      assert(waitable == 0 && code == 0);
      const status = e_my$test$i.rawExportPendingImportFuture0CancelRead(this.reader);
      assert(async_.waitableState(status) == async_.WAITABLE_CANCELLED);
      assert(async_.waitableCount(status) == 0);
      async_.waitableJoin(this.reader, 0);
      e_my$test$i.rawExportPendingImportFuture0DropReadable(this.reader);
      async_.waitableSetDrop(this.set);
      // No `finish()`: the generated finish helper must issue `task.cancel`.
      return async_.CALLBACK_CODE_EXIT;
    }

    assert(event == async_.EVENT_FUTURE_READ && waitable == this.reader);
    assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
    assert(async_.waitableCount(code) == 0);
    this.finishFuture0Read();
    async_.waitableJoin(this.reader, 0);
    e_my$test$i.rawExportPendingImportFuture0DropReadable(this.reader);
    async_.waitableSetDrop(this.set);
    return this.finish();
  }
}

export function pendingImport(x: i32): e_my$test$i.PendingImportTask {
  return new PendingImport(x);
}

export function backpressureSet(x: bool): void {
  if (x) async_.backpressureInc();
  else async_.backpressureDec();
}

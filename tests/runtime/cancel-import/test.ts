//@ [lang]
//@ path = "exports/my$test$i.ts"

import * as async_ from "../async";

@external("[export]my:test/i", "[async-lower][future-read-unit]pending-import")
declare function __read(handle: i32, payload: usize): i32;
@external("[export]my:test/i", "[future-cancel-read-unit]pending-import")
declare function __cancelRead(handle: i32): i32;
@external("[export]my:test/i", "[future-drop-readable-unit]pending-import")
declare function __dropReadable(handle: i32): void;
@external("[export]my:test/i", "[task-return]pending-import")
declare function __taskReturn(): void;

export function rawExportPendingImportFuture0Read(handle: i32, payload: usize): i32 {
  return __read(handle, payload);
}

export function rawExportPendingImportFuture0CancelRead(handle: i32): i32 {
  return __cancelRead(handle);
}

export function rawExportPendingImportFuture0DropReadable(handle: i32): void {
  __dropReadable(handle);
}

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
export class PendingImportTask extends async_.AsyncTask {
  private reader: i32;
  private set: i32 = 0;
  finished: bool = false;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (event == async_.EVENT_NONE) {
      assert(waitable == 0 && code == 0);
      assert(rawExportPendingImportFuture0Read(this.reader, 0) == -1);
      this.set = async_.waitableSetNew();
      async_.waitableJoin(this.reader, this.set);
      return async_.callbackWait(this.set);
    }

    if (event == async_.EVENT_CANCEL) {
      assert(waitable == 0 && code == 0);
      const status = rawExportPendingImportFuture0CancelRead(this.reader);
      assert(async_.waitableState(status) == async_.WAITABLE_CANCELLED);
      assert(async_.waitableCount(status) == 0);
    } else {
      assert(event == async_.EVENT_FUTURE_READ && waitable == this.reader);
      assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
      assert(async_.waitableCount(code) == 0);
      this.finished = true;
    }
    async_.waitableJoin(this.reader, 0);
    rawExportPendingImportFuture0DropReadable(this.reader);
    async_.waitableSetDrop(this.set);
    return async_.CALLBACK_CODE_EXIT;
  }
}

export function pendingImport(x: i32): PendingImportTask {
  return new PendingImportTask(x);
}

export function backpressureSet(x: bool): void {
  if (x) async_.backpressureInc();
  else async_.backpressureDec();
}

export function __exp_0_pendingImport(a0: i32): i32 {
  const status = async_.Scheduler.start(pendingImport(a0));
  __finishPendingImport(status);
  return status;
}

export function __callback_0_pendingImport(event: i32, waitable: i32, code: i32): i32 {
  const status = async_.Scheduler.resume(event, waitable, code);
  __finishPendingImport(status);
  return status;
}

function __finishPendingImport(status: i32): void {
  const task = async_.contextGet();
  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;
  if (load<bool>(task + offsetof<PendingImportTask>("finished"))) __taskReturn();
  async_.Scheduler.release(task);
  async_.Scheduler.complete(task);
}

export function __exp_0_backpressureSet(a0: i32): void {
  backpressureSet(a0 != 0);
}

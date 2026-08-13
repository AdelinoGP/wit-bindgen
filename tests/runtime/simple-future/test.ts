//@ [lang]
//@ path = "exports/my$test$i.ts"

import * as async_ from "../async";

@external("[export]my:test/i", "[async-lower][future-read-unit]read-future")
declare function __readRead(handle: i32, payload: usize): i32;
@external("[export]my:test/i", "[future-new-unit]read-future")
declare function __readNew(): i64;
@external("[export]my:test/i", "[future-drop-readable-unit]read-future")
declare function __readDrop(handle: i32): void;
@external("[export]my:test/i", "[future-drop-readable-unit]drop-future")
declare function __dropDrop(handle: i32): void;
@external("[export]my:test/i", "[future-new-unit]drop-future")
declare function __dropNew(): i64;

@unmanaged
export abstract class FTask extends async_.AsyncTask {
  finished: bool = false;
  finish(): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@unmanaged
class ReadFutureTask extends FTask {
  private reader: i32;
  private set: i32 = 0;
  private state: i32 = 0;
  constructor(reader: i32) { super(); this.reader = reader; }
  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.state == 0) {
      assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
      this.state = 1;
      const status = rawExportReadFutureFuture0Read(this.reader, 0);
      if (status == -1) {
        this.set = async_.waitableSetNew();
        async_.waitableJoin(this.reader, this.set);
        return async_.callbackWait(this.set);
      }
      assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
    } else {
      assert(event == async_.EVENT_FUTURE_READ && waitable == this.reader);
      assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
    }
    rawExportReadFutureFuture0DropReadable(this.reader);
    if (this.set != 0) async_.waitableSetDrop(this.set);
    return this.finish();
  }
}

export function readFuture(x: i32): ReadFutureTask { return new ReadFutureTask(x); }

@unmanaged
class DropFutureTask extends FTask {
  private reader: i32;
  constructor(reader: i32) { super(); this.reader = reader; }
  resume(event: i32, waitable: i32, code: i32): i32 {
    assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
    rawExportDropFutureFuture0DropReadable(this.reader);
    return this.finish();
  }
}


export function dropFuture(x: i32): DropFutureTask { return new DropFutureTask(x); }

export function rawExportReadFutureFuture0New(): i64 { return __readNew(); }
export function rawExportReadFutureFuture0Read(handle: i32, payload: usize): i32 {
  return __readRead(handle, payload);
}
export function rawExportReadFutureFuture0DropReadable(handle: i32): void { __readDrop(handle); }
export function rawExportDropFutureFuture0New(): i64 { return __dropNew(); }
export function rawExportDropFutureFuture0Read(handle: i32, payload: usize): i32 { return -1; }
export function rawExportDropFutureFuture0DropReadable(handle: i32): void { __dropDrop(handle); }

function assert(condition: bool): void { if (!condition) unreachable(); }

export function __exp_0_readFuture(a0: i32): i32 {
  const status = async_.Scheduler.start(readFuture(a0));
  __finish___exp_0_readFuture(status);
  return status;
}
export function __callback_0_readFuture(event: i32, waitable: i32, code: i32): i32 {
  const status = async_.Scheduler.resume(event, waitable, code);
  __finish___exp_0_readFuture(status);
  return status;
}
@external("[export]my:test/i", "[task-return]read-future")
declare function __returnRead(): void;
function __finish___exp_0_readFuture(status: i32): void {
  const task = async_.contextGet();
  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;
  if (load<bool>(task + offsetof<ReadFutureTask>("finished"))) __returnRead();
  async_.Scheduler.release(task);
  async_.Scheduler.complete(task);
}

export function __exp_0_dropFuture(a0: i32): i32 {
  const status = async_.Scheduler.start(dropFuture(a0));
  __finish___exp_0_dropFuture(status);
  return status;
}
export function __callback_0_dropFuture(event: i32, waitable: i32, code: i32): i32 {
  const status = async_.Scheduler.resume(event, waitable, code);
  __finish___exp_0_dropFuture(status);
  return status;
}
@external("[export]my:test/i", "[task-return]drop-future")
declare function __returnDrop(): void;
function __finish___exp_0_dropFuture(status: i32): void {
  const task = async_.contextGet();
  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;
  if (load<bool>(task + offsetof<DropFutureTask>("finished"))) __returnDrop();
  async_.Scheduler.release(task);
  async_.Scheduler.complete(task);
}

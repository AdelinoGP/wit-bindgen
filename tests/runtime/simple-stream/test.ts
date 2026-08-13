//@ [lang]
//@ path = "exports/my$test$i.ts"

import * as async_ from "../async";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
export class ReadStreamTask extends async_.AsyncTask {
  private reader: i32;
  private set: i32 = 0;
  private state: i32 = 0;
  finished: bool = false;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (this.state == 0) {
      assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);

      let status = rawExportReadStreamStream0Read(this.reader, 0, 1);
      assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
      assert(async_.waitableCount(status) == 1);

      status = rawExportReadStreamStream0Read(this.reader, 0, 2);
      assert(status == -1);
      this.state = 1;
      this.set = async_.waitableSetNew();
      async_.waitableJoin(this.reader, this.set);
      return async_.callbackWait(this.set);
    }

    assert(this.state == 1);
    assert(event == async_.EVENT_STREAM_READ && waitable == this.reader);
    assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
    assert(async_.waitableCount(code) == 2);

    async_.waitableJoin(this.reader, 0);
    rawExportReadStreamStream0DropReadable(this.reader);
    async_.waitableSetDrop(this.set);
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

export function readStream(x: i32): ReadStreamTask {
  return new ReadStreamTask(x);
}

@external("[export]my:test/i", "[stream-new-unit]read-stream")
declare function __rawExportReadStreamStream0New(): i64;
export function rawExportReadStreamStream0New(): i64 {
  return __rawExportReadStreamStream0New();
}

@external("[export]my:test/i", "[async-lower][stream-read-unit]read-stream")
declare function __rawExportReadStreamStream0Read(handle: i32, payload: usize, count: usize): i32;
export function rawExportReadStreamStream0Read(handle: i32, payload: usize, count: usize): i32 {
  return __rawExportReadStreamStream0Read(handle, payload, count);
}

@external("[export]my:test/i", "[stream-drop-readable-unit]read-stream")
declare function __rawExportReadStreamStream0DropReadable(handle: i32): void;
export function rawExportReadStreamStream0DropReadable(handle: i32): void {
  __rawExportReadStreamStream0DropReadable(handle);
}

@inline(false)
function __start___exp_0_readStream(a0: i32): i32 {
  return async_.Scheduler.start(readStream(a0));
}

export function __exp_0_readStream(a0: i32): i32 {
  const status = __start___exp_0_readStream(a0);
  __finish___exp_0_readStream(status);
  return status;
}

@external("[export]my:test/i", "[task-return]read-stream")
declare function __task_return_0_readStream(): void;

@inline(false)
export function __finish___exp_0_readStream(status: i32): void {
  const task = async_.contextGet();
  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;
  if (load<bool>(task + offsetof<ReadStreamTask>("finished"))) {
    __task_return_0_readStream();
  }
  async_.Scheduler.release(task);
  async_.Scheduler.complete(task);
}

export function __callback_0_readStream(event: i32, waitable: i32, code: i32): i32 {
  const status = async_.Scheduler.resume(event, waitable, code);
  __finish___exp_0_readStream(status);
  return status;
}

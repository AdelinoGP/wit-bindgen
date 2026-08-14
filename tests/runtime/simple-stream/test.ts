//@ [lang]
//@ path = "stubs/my$test$i.ts"

import * as async_ from "../async";
import * as e_my$test$i from "../exports/my$test$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
class ReadStream extends e_my$test$i.ReadStreamTask {
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

      let status = e_my$test$i.rawExportReadStreamStream0Read(this.reader, 0, 1);
      assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
      assert(async_.waitableCount(status) == 1);

      status = e_my$test$i.rawExportReadStreamStream0Read(this.reader, 0, 2);
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
    e_my$test$i.rawExportReadStreamStream0DropReadable(this.reader);
    async_.waitableSetDrop(this.set);
    return this.finish();
  }
}

export function readStream(x: i32): e_my$test$i.ReadStreamTask {
  return new ReadStream(x);
}

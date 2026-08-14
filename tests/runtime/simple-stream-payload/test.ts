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
  private step: i32 = 0;
  private want: i32 = 0;

  constructor(reader: i32) {
    super();
    this.reader = reader;
  }

  resume(event: i32, waitable: i32, code: i32): i32 {
    if (event != async_.EVENT_NONE) {
      assert(event == async_.EVENT_STREAM_READ && waitable == this.reader);
      assert(async_.waitableState(code) == async_.WAITABLE_COMPLETED);
      this.check(async_.waitableCount(code));
    }

    while (this.step < 4) {
      // One item, then two, then the two halves of a two-item write.
      this.want = this.step == 1 ? 2 : 1;
      this.step++;
      const status = this.startStream0Read(this.reader, this.want);
      if (status == -1) {
        if (this.set == 0) {
          this.set = async_.waitableSetNew();
          async_.waitableJoin(this.reader, this.set);
        }
        return async_.callbackWait(this.set);
      }
      assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
      this.check(async_.waitableCount(status));
    }

    if (this.set != 0) {
      async_.waitableJoin(this.reader, 0);
      async_.waitableSetDrop(this.set);
      this.set = 0;
    }
    e_my$test$i.rawExportReadStreamStream0DropReadable(this.reader);
    return this.finish();
  }

  /// The generated typed helper hands the payload back as a `Uint8Array`.
  private check(count: i32): void {
    assert(count == this.want);
    const values = this.finishStream0Read(count);
    assert(values.length == count);
    for (let i = 0; i < count; i++) {
      assert(values[i] == <u8>(this.expectedFirst() + i));
    }
  }

  /// The writer sends 0 | 1,2 | 3,4, and this task reads 1 | 2 | 1 | 1 items.
  private expectedFirst(): u8 {
    switch (this.step) {
      case 1:
        return 0;
      case 2:
        return 1;
      case 3:
        return 3;
      default:
        return 4;
    }
  }
}

export function readStream(x: i32): e_my$test$i.ReadStreamTask {
  return new ReadStream(x);
}

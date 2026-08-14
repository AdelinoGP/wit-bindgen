//@ [lang]
//@ path = "stubs/a$b$i.ts"

import * as async_ from "../async";
import * as e_a$b$i from "../exports/a$b$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

// Every task below yields once before returning. Completing synchronously in
// the first `resume` would never drive the generated `[callback]` export.

@unmanaged
class OneArgument extends e_a$b$i.OneArgumentTask {
  private yielded: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    if (!this.yielded) {
      this.yielded = true;
      return async_.Scheduler.yield();
    }
    return this.finish();
  }
}

@unmanaged
class OneResult extends e_a$b$i.OneResultTask {
  private yielded: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    if (!this.yielded) {
      this.yielded = true;
      return async_.Scheduler.yield();
    }
    return this.finish(2);
  }
}

@unmanaged
class OneArgumentAndResult extends e_a$b$i.OneArgumentAndResultTask {
  private yielded: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    if (!this.yielded) {
      this.yielded = true;
      return async_.Scheduler.yield();
    }
    return this.finish(4);
  }
}

@unmanaged
class TwoArguments extends e_a$b$i.TwoArgumentsTask {
  private yielded: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    if (!this.yielded) {
      this.yielded = true;
      return async_.Scheduler.yield();
    }
    return this.finish();
  }
}

@unmanaged
class TwoArgumentsAndResult extends e_a$b$i.TwoArgumentsAndResultTask {
  private yielded: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    if (!this.yielded) {
      this.yielded = true;
      return async_.Scheduler.yield();
    }
    return this.finish(9);
  }
}

export function oneArgument(x: u32): e_a$b$i.OneArgumentTask {
  assert(x == 1);
  return new OneArgument();
}

export function oneResult(): e_a$b$i.OneResultTask {
  return new OneResult();
}

export function oneArgumentAndResult(x: u32): e_a$b$i.OneArgumentAndResultTask {
  assert(x == 3);
  return new OneArgumentAndResult();
}

export function twoArguments(x: u32, y: u32): e_a$b$i.TwoArgumentsTask {
  assert(x == 5 && y == 6);
  return new TwoArguments();
}

export function twoArgumentsAndResult(x: u32, y: u32): e_a$b$i.TwoArgumentsAndResultTask {
  assert(x == 7 && y == 8);
  return new TwoArgumentsAndResult();
}

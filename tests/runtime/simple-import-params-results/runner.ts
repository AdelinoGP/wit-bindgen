//@ args = '--async=-run'
//@ wasmtime-flags = '-Wcomponent-model-async'
//@ [lang]
//@ path = "world.ts"

import * as async_ from "./async";
import * as i_a$b$i from "./imports/a$b$i";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
export class RunTask extends async_.AsyncTask {
  set: i32 = 0;
  first: usize = 0;
  second: usize = 0;
  pending: i32 = 0;

  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    return async_.CALLBACK_CODE_EXIT;
  }

  run(): void {
    const unit = i_a$b$i.oneArgument(1);
    assert(unit.state == async_.STATUS_RETURNED);
    unit.finish(unit.status);

    const one = i_a$b$i.oneResult();
    const two = i_a$b$i.twoArgumentsAndResult(7, 8);
    this.first = changetype<usize>(one);
    this.second = changetype<usize>(two);
    this.set = async_.waitableSetNew();
    this.pending = 2;
    if (one.handle != 0) async_.waitableJoin(one.handle, this.set);
    else { assert(one.finish(one.status) == 2); this.first = 0; this.pending--; }
    if (two.handle != 0) async_.waitableJoin(two.handle, this.set);
    else { assert(two.finish(two.status) == 9); this.second = 0; this.pending--; }

    const payload = changetype<usize>(memory.data(8));
    while (this.pending != 0) {
      assert(async_.waitableSetWait(this.set, payload) == async_.EVENT_SUBTASK);
      const waitable = load<i32>(payload);
      const code = load<i32>(payload + 4);
      if (this.first != 0 && changetype<i_a$b$i.OneResultSubtask>(this.first).handle == waitable) {
        assert(changetype<i_a$b$i.OneResultSubtask>(this.first).finish(code) == 2);
        this.first = 0;
      } else {
        assert(this.second != 0);
        assert(changetype<i_a$b$i.TwoArgumentsAndResultSubtask>(this.second).finish(code) == 9);
        this.second = 0;
      }
      this.pending--;
    }

    async_.waitableSetDrop(this.set);
    const mixed = i_a$b$i.oneArgumentAndResult(3);
    assert(mixed.state == async_.STATUS_RETURNED);
    assert(mixed.finish(mixed.status) == 4);
    const twoUnit = i_a$b$i.twoArguments(5, 6);
    assert(twoUnit.state == async_.STATUS_RETURNED);
    twoUnit.finish(twoUnit.status);
  }
}

export function run(): void { new RunTask().run(); }

export function __exp_18446744073709551615_run(): void { run(); }

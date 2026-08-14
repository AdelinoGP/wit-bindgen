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

/// The callee re-acquires argument handles while cancelling, so a cancelled
/// import must leave its future's write end dropped.
function assertDropped(handle: i32): void {
  const status = i_my$test$i.rawImportPendingImportFuture0Write(handle, 0);
  assert(async_.waitableState(status) == async_.WAITABLE_DROPPED);
  assert(async_.waitableCount(status) == 0);
  i_my$test$i.rawImportPendingImportFuture0DropWritable(handle);
}

@unmanaged
class RunTask extends world.RunTask {
  // Only scalars survive a yield: subtasks are `@unmanaged`, so keep raw
  // pointers to them.
  private step: i32 = 0;
  private yields: i32 = 0;
  private pendingA: usize = 0;
  private pendingB: usize = 0;
  private writerA: i32 = 0;
  private writerB: i32 = 0;
  private set: i32 = 0;

  resume(event: i32, waitable: i32, code: i32): i32 {
    while (true) {
      switch (this.step) {
        case 0:
          assert(event == async_.EVENT_NONE && waitable == 0 && code == 0);
          this.cancelScenarios();
          this.step = 1;
          continue;
        case 1:
          this.startCompletedRace();
          this.yields = 0;
          this.step = 2;
          return async_.Scheduler.yield();
        case 2:
          if (++this.yields < 5) return async_.Scheduler.yield();
          this.finishCompletedRace();
          this.step = 3;
          continue;
        case 3:
          this.startTransitionRace();
          this.yields = 0;
          this.step = 4;
          return async_.Scheduler.yield();
        case 4:
          if (++this.yields < 5) return async_.Scheduler.yield();
          this.step = 5;
          return this.finishTransitionRace();
        case 5: {
          const subtask = changetype<i_my$test$i.PendingImportSubtask>(this.pendingA);
          assert(event == async_.EVENT_SUBTASK && waitable == subtask.handle);
          assert(async_.subtaskState(code) == async_.STATUS_RETURNED);
          this.pendingA = 0;
          subtask.finish(code);
          async_.waitableSetDrop(this.set);
          this.set = 0;
          return this.finish();
        }
        default:
          unreachable();
          return async_.CALLBACK_CODE_EXIT;
      }
    }
  }

  /// Race an import's cancellation against a status code saying it is done.
  private startCompletedRace(): void {
    const pair = i_my$test$i.rawImportPendingImportFuture0New();
    const subtask = i_my$test$i.pendingImport(reader(pair));
    assert(subtask.state == async_.STATUS_STARTED);
    this.pendingA = changetype<usize>(subtask);
    // Let the callee complete, but do not observe the completion yet.
    const status = i_my$test$i.rawImportPendingImportFuture0Write(writer(pair), 0);
    assert(async_.waitableState(status) == async_.WAITABLE_COMPLETED);
    i_my$test$i.rawImportPendingImportFuture0DropWritable(writer(pair));
  }

  private finishCompletedRace(): void {
    const subtask = changetype<i_my$test$i.PendingImportSubtask>(this.pendingA);
    this.pendingA = 0;
    const status = subtask.cancel();
    const state = async_.subtaskState(status);
    assert(state == async_.STATUS_RETURNED || state == async_.STATUS_RETURNED_CANCELLED);
    subtask.dispose(status);
  }

  /// Race an import's cancellation against a queued STARTING => STARTED
  /// transition.
  private startTransitionRace(): void {
    const pair1 = i_my$test$i.rawImportPendingImportFuture0New();
    const started = i_my$test$i.pendingImport(reader(pair1));
    assert(started.state == async_.STATUS_STARTED);
    this.pendingA = changetype<usize>(started);
    this.writerA = writer(pair1);

    i_my$test$i.backpressureSet(true);
    const pair2 = i_my$test$i.rawImportPendingImportFuture0New();
    const starting = i_my$test$i.pendingImport(reader(pair2));
    assert(starting.state == async_.STATUS_STARTING);
    this.pendingB = changetype<usize>(starting);
    // Releasing backpressure queues up the STARTING => STARTED notification.
    i_my$test$i.backpressureSet(false);
    this.writerB = writer(pair2);
  }

  private finishTransitionRace(): i32 {
    const starting = changetype<i_my$test$i.PendingImportSubtask>(this.pendingB);
    this.pendingB = 0;
    const status = starting.cancel();
    const state = async_.subtaskState(status);
    assert(
      state == async_.STATUS_STARTED_CANCELLED || state == async_.STATUS_RETURNED_CANCELLED
    );
    assert(starting.dispose(status));
    assertDropped(this.writerB);

    // The untouched import must still be able to proceed normally.
    const started = changetype<i_my$test$i.PendingImportSubtask>(this.pendingA);
    const write = i_my$test$i.rawImportPendingImportFuture0Write(this.writerA, 0);
    assert(async_.waitableState(write) == async_.WAITABLE_COMPLETED);
    i_my$test$i.rawImportPendingImportFuture0DropWritable(this.writerA);
    this.set = async_.waitableSetNew();
    async_.waitableJoin(started.handle, this.set);
    return async_.callbackWait(this.set);
  }

  private cancelScenarios(): void {
    // Cancel an import that is in progress (STARTED).
    {
      const pair = i_my$test$i.rawImportPendingImportFuture0New();
      const started = i_my$test$i.pendingImport(reader(pair));
      assert(started.state == async_.STATUS_STARTED);
      assert(started.handle != 0);
      const status = started.cancel();
      assert(async_.subtaskState(status) == async_.STATUS_RETURNED_CANCELLED);
      assert(async_.subtaskHandle(status) == 0);
      assert(started.dispose(status));
      assertDropped(writer(pair));
    }

    // Cancel an import before it starts (STARTING, held back by backpressure).
    {
      const pair = i_my$test$i.rawImportPendingImportFuture0New();
      i_my$test$i.backpressureSet(true);
      const starting = i_my$test$i.pendingImport(reader(pair));
      assert(starting.state == async_.STATUS_STARTING);
      assert(starting.handle != 0);
      const status = starting.cancel();
      assert(async_.subtaskState(status) == async_.STATUS_STARTED_CANCELLED);
      assert(async_.subtaskHandle(status) == 0);
      assert(starting.dispose(status));
      assertDropped(writer(pair));
      i_my$test$i.backpressureSet(false);
    }

    // Cancel a STARTING import while another is already STARTED, then cancel
    // the STARTED one too. Both futures must come back dropped.
    {
      const pair1 = i_my$test$i.rawImportPendingImportFuture0New();
      const started = i_my$test$i.pendingImport(reader(pair1));
      assert(started.state == async_.STATUS_STARTED);

      i_my$test$i.backpressureSet(true);
      const pair2 = i_my$test$i.rawImportPendingImportFuture0New();
      const starting = i_my$test$i.pendingImport(reader(pair2));
      assert(starting.state == async_.STATUS_STARTING);

      const startingStatus = starting.cancel();
      assert(async_.subtaskState(startingStatus) == async_.STATUS_STARTED_CANCELLED);
      assert(starting.dispose(startingStatus));

      const startedStatus = started.cancel();
      assert(async_.subtaskState(startedStatus) == async_.STATUS_RETURNED_CANCELLED);
      assert(started.dispose(startedStatus));

      i_my$test$i.backpressureSet(false);
      assertDropped(writer(pair1));
      assertDropped(writer(pair2));
    }
  }
}

export function run(): world.RunTask {
  return new RunTask();
}

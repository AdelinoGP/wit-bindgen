//@ [lang]
//@ path = "exports/a$b$i.ts"

import * as async_ from "../async";

function assert(condition: bool): void {
  if (!condition) unreachable();
}

@unmanaged
export class OneArgumentTask extends async_.AsyncTask {
  finished: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@unmanaged
export class OneResultTask extends async_.AsyncTask {
  finished: bool = false;
  result: u32;
  constructor(result: u32) { super(); this.result = result; }
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@unmanaged
export class OneArgumentAndResultTask extends async_.AsyncTask {
  finished: bool = false;
  result: u32;
  constructor(result: u32) { super(); this.result = result; }
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@unmanaged
export class TwoArgumentsTask extends async_.AsyncTask {
  finished: bool = false;
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

@unmanaged
export class TwoArgumentsAndResultTask extends async_.AsyncTask {
  finished: bool = false;
  result: u32;
  constructor(result: u32) { super(); this.result = result; }
  resume(_event: i32, _waitable: i32, _code: i32): i32 {
    this.finished = true;
    return async_.CALLBACK_CODE_EXIT;
  }
}

export function oneArgument(x: u32): OneArgumentTask { assert(x == 1); return new OneArgumentTask(); }
export function oneResult(): OneResultTask { return new OneResultTask(2); }
export function oneArgumentAndResult(x: u32): OneArgumentAndResultTask { assert(x == 3); return new OneArgumentAndResultTask(4); }
export function twoArguments(x: u32, y: u32): TwoArgumentsTask { assert(x == 5 && y == 6); return new TwoArgumentsTask(); }
export function twoArgumentsAndResult(x: u32, y: u32): TwoArgumentsAndResultTask { assert(x == 7 && y == 8); return new TwoArgumentsAndResultTask(9); }

@external("[export]a:b/i", "[task-return]one-argument") declare function __returnOneArgument(): void;
@external("[export]a:b/i", "[task-return]one-result") declare function __returnOneResult(value: i32): void;
@external("[export]a:b/i", "[task-return]one-argument-and-result") declare function __returnOneArgumentAndResult(value: i32): void;
@external("[export]a:b/i", "[task-return]two-arguments") declare function __returnTwoArguments(): void;
@external("[export]a:b/i", "[task-return]two-arguments-and-result") declare function __returnTwoArgumentsAndResult(value: i32): void;

function finishOneArgument(status: i32): void { const p = async_.contextGet(); if (status == 0 && p != 0) { __returnOneArgument(); async_.Scheduler.release(p); async_.Scheduler.complete(p); } }
function finishOneResult(status: i32): void { const p = async_.contextGet(); if (status == 0 && p != 0) { __returnOneResult(load<i32>(p + offsetof<OneResultTask>("result"))); async_.Scheduler.release(p); async_.Scheduler.complete(p); } }
function finishOneArgumentAndResult(status: i32): void { const p = async_.contextGet(); if (status == 0 && p != 0) { __returnOneArgumentAndResult(load<i32>(p + offsetof<OneArgumentAndResultTask>("result"))); async_.Scheduler.release(p); async_.Scheduler.complete(p); } }
function finishTwoArguments(status: i32): void { const p = async_.contextGet(); if (status == 0 && p != 0) { __returnTwoArguments(); async_.Scheduler.release(p); async_.Scheduler.complete(p); } }
function finishTwoArgumentsAndResult(status: i32): void { const p = async_.contextGet(); if (status == 0 && p != 0) { __returnTwoArgumentsAndResult(load<i32>(p + offsetof<TwoArgumentsAndResultTask>("result"))); async_.Scheduler.release(p); async_.Scheduler.complete(p); } }

export function __exp_0_oneArgument(a0: i32): i32 { const s = async_.Scheduler.start(oneArgument(<u32>a0)); finishOneArgument(s); return s; }
export function __callback_0_oneArgument(e: i32, w: i32, c: i32): i32 { const s = async_.Scheduler.resume(e, w, c); finishOneArgument(s); return s; }
export function __exp_0_oneResult(): i32 { const s = async_.Scheduler.start(oneResult()); finishOneResult(s); return s; }
export function __callback_0_oneResult(e: i32, w: i32, c: i32): i32 { const s = async_.Scheduler.resume(e, w, c); finishOneResult(s); return s; }
export function __exp_0_oneArgumentAndResult(a0: i32): i32 { const s = async_.Scheduler.start(oneArgumentAndResult(<u32>a0)); finishOneArgumentAndResult(s); return s; }
export function __callback_0_oneArgumentAndResult(e: i32, w: i32, c: i32): i32 { const s = async_.Scheduler.resume(e, w, c); finishOneArgumentAndResult(s); return s; }
export function __exp_0_twoArguments(a0: i32, a1: i32): i32 { const s = async_.Scheduler.start(twoArguments(<u32>a0, <u32>a1)); finishTwoArguments(s); return s; }
export function __callback_0_twoArguments(e: i32, w: i32, c: i32): i32 { const s = async_.Scheduler.resume(e, w, c); finishTwoArguments(s); return s; }
export function __exp_0_twoArgumentsAndResult(a0: i32, a1: i32): i32 { const s = async_.Scheduler.start(twoArgumentsAndResult(<u32>a0, <u32>a1)); finishTwoArgumentsAndResult(s); return s; }
export function __callback_0_twoArgumentsAndResult(e: i32, w: i32, c: i32): i32 { const s = async_.Scheduler.resume(e, w, c); finishTwoArgumentsAndResult(s); return s; }

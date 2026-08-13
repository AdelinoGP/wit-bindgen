# AssemblyScript Async Backend Handoff

## Current State

Workspace: `D:\wit-bindgen`.

Latest committed changes are `af836ef2` (async ownership cleanup and typed
error-context support) and `99076ead` (async endpoint alias matching).

This handoff is intentionally uncommitted. Do not reset, clean, or revert the worktree. It contains unrelated concurrent changes and diagnostic artifacts.

## Objective

Complete the AssemblyScript async backend: executable async exports/imports, future and stream endpoints, error-context support, typed ownership, cancellation, tests, and both AssemblyScript runtimes (`incremental` and `minimal`).

## Committed Milestones

- `af0aa14b`: async callback ABI foundation.
- `7dbb24c7`: async endpoint task support, raw future/stream helpers, error-context helper scaffolding, generated task bases.
- `ff87bd87`: executable async yield exports, unmanaged task state, captured-pointer cleanup, and `simple-yield`.
- `082fb52e`: typed future task helpers for generated async export task bases.
- `3c702df0`: executable future endpoints and the unmanaged task dispatch fix. The scheduler captures a concrete method thunk/index and resumes via `call_indirect`; virtual dispatch through an unmanaged base is unsafe.
- `8c84e135`: canonical endpoint cancel names. Only future/stream `read` and `write` use `[async-lower]`; cancel intrinsics do not.
- `ee13dba4`: executable stream endpoints and `simple-stream`.
- `284c965d`: per-call unmanaged async-import subtasks with owned scalar result areas; migrated yield/future/stream runners; added `simple-import-params-results`.
- `e39bbf2e`: typed async-import cancellation, cancellation cleanup, endpoint `DropHandle` cleanup, and `cancel-import` fixtures.
- `af836ef2`: typed owned `ErrorContext`, exact-once cancellation and
  reentrancy guards, imported/exported owned-resource cleanup, and regression
  tests.
- `99076ead`: async future/stream endpoint cleanup matching across aliases and
  duplicate occurrences.

## Validated Behavior

Both AssemblyScript runtimes passed mixed Rust/AssemblyScript matrices for:

- `simple-yield`.
- `simple-future`.
- `simple-stream`.
- `simple-import-params-results`, including concurrent scalar result imports.
- `cancel-import`, including `STATUS_RETURNED_CANCELLED` and `STATUS_STARTED_CANCELLED`.

Representative commands:

```text
cargo run test --languages rust,assemblyscript tests/runtime/simple-yield tests/runtime/simple-future tests/runtime/simple-stream tests/runtime/simple-import-params-results --artifacts target/artifacts-as-owned-final --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript tests/runtime/simple-yield tests/runtime/simple-future tests/runtime/simple-stream tests/runtime/simple-import-params-results --artifacts target/artifacts-as-owned-final-minimal --rust-wit-bindgen-path ./crates/guest-rust
cargo run test --languages rust,assemblyscript tests/runtime/cancel-import --artifacts target/artifacts-cancel-as-final --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript tests/runtime/cancel-import --artifacts target/artifacts-cancel-as-final-minimal --rust-wit-bindgen-path ./crates/guest-rust
```

Rust checks passed after the committed slices:

```text
cargo fmt --check
cargo test -p wit-bindgen-assemblyscript
cargo clippy -p wit-bindgen-assemblyscript --all-targets -- -D warnings
git diff --check
```

Full AssemblyScript codegen passes all `212` tests. Async named fixed-length
lists now deallocate each flattened element through the shared core ABI.

## Uncommitted Tracked Changes

At handoff, tracked changes are:

- `crates/assemblyscript/src/async/async.ts`.
- `crates/assemblyscript/src/lib.rs`.
- `crates/c/README.md`.
- `crates/c/src/lib.rs`.

The C changes are unrelated concurrent work. Do not stage or modify them.

The C changes are unrelated concurrent work. The AssemblyScript changes are:

- `Type::ErrorContext` now marks the generated world as requiring `async.ts`, while its type representation remains raw `i32`.
- Generated async-import subtasks have a `cancellationRequested` flag. `cancel()` sets it before calling `[subtask-cancel]` and rejects repeated calls locally; the original handle remains available for the required later `[subtask-drop]`.
- `Scheduler.start` traps when context-0 is already occupied. Nested scheduler tasks cannot be safely saved/restored because callback arguments carry no task identity.
- Unit tests `async_import_cancel_is_exactly_once` and `scheduler_rejects_reentrant_tasks` pin the new behavior.

The endpoint occurrence fix, already committed in `e39bbf2e`, is:

- `FunctionBindgen` has `next_endpoint`.
- `DropHandle` consumes future/stream occurrences in canonical function order instead of finding the first matching `TypeId`.
- Async import result cleanup starts its endpoint cursor after parameter occurrences.
- Unit test `async_import_drop_helpers_follow_duplicate_endpoint_occurrences` checks duplicate parameter endpoints and a later result endpoint.

The uncommitted AssemblyScript diff passed unit tests, formatting, clippy, focused cancellation matrices under both runtimes, and `git diff --check`. It has not been committed. Do not stage the unrelated C changes.

## Design Constraints

### Unmanaged export tasks

Generated/user async export task hierarchies are `@unmanaged`. Persist only scalar fields, raw pointers, handles, booleans, and integer state. Do not persist managed strings, arrays, records, or class references across a yield.

AssemblyScript virtual dispatch reads a managed runtime type header. Calling an overridden method through an unmanaged base is unsafe. The scheduler fix in `3c702df0` captures the concrete method reference/index in `AsyncTask.resumeIndex` at `Scheduler.start` and resumes via `call_indirect`. Preserve this design unless replacing it with explicit concrete per-export dispatch.

### Async-import subtasks

Generated async imports return a per-call `@unmanaged` concrete `*Subtask`, not a raw status. It owns status/state/handle, a result-area pointer, lowered scalar parameters, and lifecycle flags.

Typical use:

```text
const subtask = imports.someAsync(...);
const value = subtask.finish(status); // only STATUS_RETURNED
// or subtask.dispose(status) for cancellation/discarding a result
```

`cancel()` invokes canonical `[subtask-cancel]`, updates the object status, and returns the new status. `finish`/`dispose` are intended to be exact-once operations and free the unmanaged subtask after cleanup.

### Endpoint DropHandle

Core ABI `Instruction::DropHandle` carries only a type, not an explicit endpoint occurrence. The AssemblyScript backend now tracks occurrence order with `next_endpoint`. Preserve the duplicate-type regression test and verify parameter occurrences precede result occurrences.

## Known Gaps

### Managed async import results

Scalar async import results work. Managed result values currently do not have a validated incremental-runtime implementation.

Untracked experiments exist at `tests/runtime/return-string/runner.ts` and `tests/runtime/return-string/test.rs`. They repeatedly import an async string result.

Observed behavior:

- The fixture compiles.
- Minimal has passed in some runs.
- Incremental traps in AssemblyScript runtime/GC code.
- Copying the canonical string into a managed `Uint16Array`/`ArrayBuffer` before decoding did not fix the trap.
- Removing individual cleanup operations did not remove the trap.
- The failure is likely managed allocation/lifting after the canonical async call and/or result ownership sequencing, not simply UTF-16 encoding.

Do not commit the return-string fixture or an unvalidated string-lift change. The committed `ffi.strLift` remains the earlier `decodeUnsafe` implementation.

### Error context

Runtime helper scaffolding exists in `crates/assemblyscript/src/async/async.ts`: `ErrorContext.new`, `debugMessage`, `drop`, and raw `errorContextNew`, `errorContextDebugMessage`, and `errorContextDrop` helpers.

Generated WIT `error-context` now maps to `async_.ErrorContext` in
`crates/assemblyscript/src/lib.rs`. Lowering uses `.handle`, lifting constructs
`new async_.ErrorContext(handle)`, and owned cleanup calls `.drop()`. Generator
coverage exists, but no stable runtime coverage exists yet.

The fixture was evaluated after this handoff. Its synchronous WIT form is only an experiment: changing it to async and supplying the required hand-written AssemblyScript task wrappers compiles, but execution under both `incremental` and `minimal` fails with `unknown handle index 1` after enabling both `-Wcomponent-model-async` and `-Wcomponent-model-error-context`. Do not commit the wrapper experiment. The failure confirms that raw handle transfer cannot provide the required ownership semantics.

The paused `tests/runtime/error-context/` experiment compiles but traps during
execution under both runtime selections, including a synchronous reduction.
Do not claim runtime support until round-trip, creation, and drop pass under
both runtimes.

### Typed endpoint payloads

Raw future/stream endpoint helpers and unit future/stream runtime execution work. Generated typed future helpers are export-side only. Streams have no typed payload helper API. No AssemblyScript payload runtime fixture exists.

### Reentrancy

`Scheduler.start` now rejects a nested start when context-0 is occupied. A save/restore stack is not correct because callback arguments carry no task identity: if both parent and child suspend, either restored pointer can route the next callback to the wrong task. Unit test `scheduler_rejects_reentrant_tasks` pins the runtime guard. No end-to-end trap fixture exists yet; add one if the test harness gains expected-failure runtime support.

### Owned resources on cancellation

Imported `own<T>` cleanup reconstructs the wrapper from its raw handle and calls
`.drop()`. Exported resources maintain a reverse instance-to-handle map and
have instance-based drop dispatch. Generator tests cover both paths, and the
established async matrices pass under both runtimes.

### Full codegen

The only observed full-codegen failure is the shared core ABI `todo!()` for async named fixed-length lists at `crates/core/src/abi.rs:2486`.

## Untracked Worktree Content

Many diagnostic artifacts are present. Do not delete them unless explicitly requested or ownership is confirmed:

- malformed `C...Temp...` files;
- `artifacts-as-*`, `artifacts-c-simple`, and `artifacts-review-async-import`;
- `nul`;
- `return.wat`;
- untracked `tests/runtime/return-string/runner.ts` and `test.rs`;
- untracked `tests/runtime/error-context/`.

Stashes:

- `stash@{0}`: `wip global async import result slots` (rejected because global result slots overwrite concurrent ownership).
- `stash@{1}`: `wip opaque async task ids` (rejected because it leaked/retained tasks and did not solve incremental behavior).

Do not apply or drop either stash blindly.

## Recommended Next Steps

1. Diagnose managed async result lifting with a minimal string progression. The return-string experiment compiles, minimal passes, and incremental traps in AssemblyScript GC/runtime code. Do not commit an unvalidated string-lift change; incremental is the required gate.
2. Isolate the error-context runtime trap under both runtime selections.
4. Repeated cancellation is now rejected locally: generated subtasks set `cancellationRequested` before invoking `[subtask-cancel]`, while retaining the handle for the one required `[subtask-drop]` during `dispose`/`finish`. Unit test `async_import_cancel_is_exactly_once` pins the emitted guard. The existing cancellation fixture passes under both runtime selections. A runtime test cannot safely invoke the second call because the canonical ABI requires it to trap.
5. Reentrancy is now explicitly rejected: `Scheduler.start` traps when context-0 is occupied because callback arguments do not identify a task and save/restore would misroute suspended parent/child callbacks. Unit test `scheduler_rejects_reentrant_tasks` pins the guard. No end-to-end trap fixture was added because the runtime harness has no established expected-failure fixture path. The established async matrices pass under both runtime selections with this guard.
6. Decide whether typed stream/future payload APIs are in scope. Current stream support exposes raw pointer/length payload access; no AssemblyScript payload runtime fixture exists.
7. Add a dedicated owned-resource cancellation fixture if runtime ownership coverage is required.
8. Re-run full codegen after future ABI changes; no known AssemblyScript
   codegen failures remain.

## Useful Files

- Generator: `crates/assemblyscript/src/lib.rs`.
- Embedded runtime: `crates/assemblyscript/src/async/async.ts` and `crates/assemblyscript/src/async.rs`.
- FFI: `crates/assemblyscript/src/ffi/ffi.ts`.
- Endpoint generation: `emit_future_stream_helpers`, `emit_typed_future_helpers`, `emit_async_import_subtask`.
- Task generation: `emit_async_task_base`, `emit_async_export_support`.
- Runtime fixtures: `tests/runtime/simple-yield`, `simple-future`, `simple-stream`, `simple-import-params-results`, `cancel-import`.
- Canonical implementations: `crates/guest-rust/src/rt/async_support`, `crates/moonbit/src/async_support.rs`, `crates/c/src/lib.rs`.

## Validation Policy

Before future commits:

```text
cargo fmt --check
cargo test -p wit-bindgen-assemblyscript
cargo clippy -p wit-bindgen-assemblyscript --all-targets -- -D warnings
git diff --check
```

For async behavior, always run both runtime selections:

```text
cargo run test --languages rust,assemblyscript <focused tests> --artifacts <incremental artifacts> --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript <focused tests> --artifacts <minimal artifacts> --rust-wit-bindgen-path ./crates/guest-rust
```

Never stage `crates/c/*`, generated diagnostic artifacts, `nul`, or return-string experiments unless the next task explicitly owns them.

# AssemblyScript Async Backend Handoff

## Current State

Workspace: `D:\wit-bindgen`.

This document was rewritten after an adversarial review of every commit since
`7942297b`. The previous revision overstated what was validated; the corrections
are recorded in "Corrected Claims" below. Treat that section as the primary
result of the review.

## Objective

Complete the AssemblyScript async backend: executable async exports/imports,
future and stream endpoints, error-context support, typed ownership,
cancellation, tests, and both AssemblyScript runtimes (`incremental` and
`minimal`).

## Blocking Environment Issue

`node`, `npm`, and therefore `asc` are **not available in this environment**.
They were present earlier in the session (`node v26.2.0`, `npm 11.13.0`) and
disappeared mid-session: both vfox Node SDK directories
(`~/.vfox/cache/nodejs/v-26.2.0`, `v-25.8.2`) contain no binaries, and there is
no system Node install.

Consequences:

- No AssemblyScript runtime fixture can be built or executed.
- No AssemblyScript **codegen** test can run either: verification shells out to
  `asc --noEmit`, and `prepare()` hard-fails when `asc` is missing.
- The generator changes described below are covered by Rust unit tests that
  assert on emitted source text. **They have not been type-checked by `asc`.**

Restore Node/AssemblyScript before trusting any AssemblyScript result.

## Corrected Claims

The following statements in the previous handoff were false or unsupported.

### The runtime fixtures do not test generated async export code

`crates/test/src/assemblyscript.rs` copies each fixture over the generated file
named by its `//@ path`. Every async `test.ts` sets
`path = "exports/<iface>.ts"`, so the fixture **replaces the entire generated
interface file** — task base, `__exp_*`, `__callback_*`, `__finish___exp_*`, and
the `@external` endpoint declarations.

The fixtures therefore exercise hand-written replicas of the generator's output,
not the generator. Two independent confirmations:

- `tests/runtime/simple-future/test.ts` and `simple-stream/test.ts` hand-declare
  their own `[future-read-*]`/`[stream-read-*]` intrinsics, bypassing
  `emit_future_stream_helpers`.
- Changing the shared runtime's cancellation protocol broke `cancel-import`
  while every generated-code unit test stayed green, because the fixture carries
  its own copy of the finish helper.

Additionally, every `runner.ts` passes `//@ args = '--async=-run'`, lowering the
export under test synchronously. **No generated AssemblyScript async export is
executed by any runtime test.**

By contrast the Rust fixtures `include!(env!("BINDINGS"))` and implement only the
trait, so they do exercise generated code.

### AssemblyScript coverage is far narrower than the pass counts suggest

6 of 60 `tests/runtime/*` directories contain any `.ts` file. Missing fixtures
are silently skipped, never failed (`crates/test/src/lib.rs` discovery only hard
-fails when no runner or no test exists at all, which `runner.rs`/`test.rs`
satisfy). CI runs `--languages rust,c,assemblyscript tests/runtime` and reports
green regardless.

`tests/runtime/cancel-import/runner.ts` implements 2 of the 5 scenarios that
`runner.rs` covers; the two cancellation *races* are unimplemented in
AssemblyScript.

### Several generator unit tests were vacuous

- `exported_owned_resource_cleanup_uses_instance_table` asserted substrings that
  are unconditional per-resource boilerplate; it passed on a world with no async
  and no owned params.
- `error_context_uses_typed_wrapper_and_drop` asserted `.handle`, which `async.ts`
  alone contains 14 times, and never asserted a drop.
- `async_import_cleanup_drops_imported_owned_resources` asserted `"drop();"`,
  which matched the malformed double-wrapped form.

These have been strengthened or replaced.

### Worktree/stash claims were stale

The "Uncommitted Tracked Changes" and "Untracked Worktree Content" sections
described state that no longer existed: the AssemblyScript changes were
committed, both stashes were gone, and the `return-string` / `error-context`
experiments had been deleted.

### `ef8c43e3` left the codegen suite red

That commit fixed the shared-ABI `todo!()` for async fixed-length lists but left
the matching `should_fail_verify` markers in `crates/test/src/rust.rs` and
`crates/test/src/moonbit.rs`, so `named-fixed-length-list.wit-async` failed with
"test should have failed, but passed". Both markers are now removed and the
rust/moonbit/c codegen suite passes (1378 tests).

## Fixed In This Pass

All covered by `cargo test -p wit-bindgen-assemblyscript` (18 tests).

- **Owned-handle cleanup double-wrapped the operand.** `abi::deallocate` lifts an
  owned handle before emitting `DropHandle`, so the operand is already the
  wrapper; the backend emitted `new Item(new Item(h)).drop()`, passing the
  address of a heap wrapper to `[resource-drop]`. `DropHandle` now drops the
  lifted operand directly.
- **Variant deallocation emitted nothing.** `GuestDeallocateVariant` popped every
  payload block and emitted no tag switch, so `option<T>`/`result<_, T>`/`variant`
  leaked their active payload and never dropped owned handles inside it. It now
  emits the `switch` over the tag.
- **List/map deallocation dropped the element block** and passed the element
  *count* as the buffer's byte size. Both now emit the per-element loop and free
  `len * elem_size` with the element alignment.
- **Cancellation could issue two completions.** `Scheduler.resume` called
  `task.cancel` eagerly on `EVENT_CANCEL` and then resumed the task anyway, so a
  task that completed during cancellation issued both `task.cancel` and
  `task.return` and trapped; a task returning YIELD/WAIT from its cancel branch
  also trapped and leaked. The scheduler now records the cancel, dispatches to
  the task for cleanup, and forces `EXIT`; the generated finish helper issues
  whichever of `task.return`/`task.cancel` the task did not, and traps on an exit
  that is neither.
- **Use-after-free ordering.** The finish helper freed the task before clearing
  context-0 (`release` then `complete`); now reversed.
- **`threadYield()` discarded the cancellation result** from `[thread-yield]`,
  making cancellation unobservable at yield points. It now returns `bool`.
- **`ErrorContext.debugMessage` decoded before its null check**, reading address 0
  when the canonical call returned a zero pointer with a non-zero length.
- **`subtaskHandle`/`waitableCount` used arithmetic shifts** on unsigned packed
  fields, sign-extending values at or above 2^27.
- **Removed the dead managed `AsyncSubtask` class**, which contradicted the
  `@unmanaged` contract documented in the same file and was superseded by the
  per-function generated subtask.

`tests/runtime/cancel-import/test.ts` was updated to match the corrected
cancellation protocol. It could not be executed (see the environment issue).

## Open Bugs

### Exported resources are not routed through the instance table

For a resource the guest exports that is passed out through an imported async
function, the generator resolves the `use`d resource to an **imported** wrapper
class and lowers it as a raw `.handle`, instead of acquiring a handle with
`__Item_take` and releasing it with `__Item_drop_instance`. The exported instance
table is emitted but never called on this path.

Pinned by `exported_resource_through_async_import_is_not_yet_routed_through_the_instance_table`,
which asserts the *current* behaviour and is written to fail once this is fixed.

For a plain async **export** taking `own<item>`, no `DropHandle` is emitted at
all, so the table entry leaks. Owned-handle cleanup only runs on the async import
path.

### No post-return support

The AssemblyScript backend implements no post-return. `GuestDeallocate*` is
reachable only through async-import cleanup, so synchronous exports never release
owned parameters.

### Error contexts are never dropped

No `drop()` is emitted for an owned incoming `error-context`; the generated
export lifts it and returns `.handle` without releasing it.

### Managed async import results (unchanged, undiagnosed)

Scalar async import results work. Managed results were reported to trap under
`incremental`. The experiment fixtures no longer exist and could not be recreated
without Node.

A concrete, untested hypothesis from the review: the generated `{X}Subtask`
frees itself — `cleanup()` ends with `heap.free(changetype<usize>(this))` while
`finish()` is still executing and the caller still holds the raw pointer, with
nothing nulling it. AssemblyScript's TLSF stores free-list links at payload
offsets 0 and 4, which are exactly the `status` and `state` fields that
`update()` writes, so post-free writes would corrupt the allocator free list;
`incremental` would then trap on the next `__new` (which a managed lift performs
and a scalar path never does) while `minimal` never sweeps. This also explains
why the recorded workarounds (copying into a `Uint16Array`, removing individual
cleanup calls) had no effect.

Cheapest falsification: remove the self-`heap.free`, accept the leak, and re-run
under `incremental`. Requires Node.

### Typed endpoint payloads

Typed future helpers remain export-side only; streams have no typed payload API.
`tests/runtime/simple-stream-payload/` has no `.ts` fixture.

## Design Constraints

### Unmanaged export tasks

Generated/user async export task hierarchies are `@unmanaged`. Persist only
scalar fields, raw pointers, handles, booleans, and integer state. Do not persist
managed strings, arrays, records, or class references across a yield.

AssemblyScript virtual dispatch reads a managed runtime type header, so calling
an overridden method through an unmanaged base is unsafe. The scheduler captures
the concrete method reference in `AsyncTask.resumeIndex` at `Scheduler.start` and
resumes via `call_indirect`. This is empirically exercised by the fixtures, which
subclass the generated base and override `resume`.

### Async-import subtasks

Generated async imports return a per-call `@unmanaged` concrete `*Subtask`
owning status/state/handle, a result-area pointer, lowered scalar parameters, and
lifecycle flags.

```text
const subtask = imports.someAsync(...);
const value = subtask.finish(status); // only STATUS_RETURNED
// or subtask.dispose(status) for cancellation/discarding a result
```

### Endpoint DropHandle

Core ABI `Instruction::DropHandle` carries only a type, not an endpoint
occurrence, so the backend tracks occurrence order with `next_endpoint`. Preserve
the duplicate-type regression test and verify parameter occurrences precede
result occurrences.

### Reentrancy

`Scheduler.start` traps when context-0 is occupied. A save/restore stack is not
correct because callback arguments carry no task identity.

## Recommended Next Steps

1. **Restore Node/AssemblyScript**, then re-run the async matrices under both
   `incremental` and `minimal`. Nothing about the AssemblyScript runtime should
   be trusted until this happens, including the fixes in this pass.
2. **Stop letting fixtures replace generated files.** Have the async fixtures
   implement only the task subclass and consume the generated export glue, and
   drop `--async=-run` from the runners, so generated async exports are actually
   executed. This is the single highest-value change: it is what allowed the
   defects above to survive a "fully validated" handoff.
3. Fix exported-resource routing through the instance table.
4. Implement post-return so synchronous exports release owned parameters.
5. Emit `drop()` for owned error contexts, then add a runtime fixture.
6. Test the self-free hypothesis for managed async import results.

## Useful Files

- Generator: `crates/assemblyscript/src/lib.rs`.
- Embedded runtime: `crates/assemblyscript/src/async/async.ts` and
  `crates/assemblyscript/src/async.rs`.
- FFI: `crates/assemblyscript/src/ffi/ffi.ts`.
- Endpoint generation: `emit_future_stream_helpers`, `emit_typed_future_helpers`,
  `emit_async_import_subtask`.
- Task generation: `emit_async_task_base`, `emit_async_export_support`.
- Harness: `crates/test/src/assemblyscript.rs`.
- Canonical implementations: `crates/guest-rust/src/rt/async_support`,
  `crates/moonbit/src/async_support.rs`, `crates/c/src/lib.rs`.

## Validation Policy

```text
cargo fmt --check
cargo test -p wit-bindgen-assemblyscript
cargo clippy -p wit-bindgen-assemblyscript --all-targets -- -D warnings
cargo run test --languages rust,moonbit,c tests/codegen --artifacts <dir> --wasi-sdk-path <sdk>
git diff --check
```

For async behavior, always run both runtime selections (requires Node):

```text
cargo run test --languages rust,assemblyscript <tests> --artifacts <dir> --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript <tests> --artifacts <dir> --rust-wit-bindgen-path ./crates/guest-rust
```

Never stage `crates/c/*` — it holds unrelated concurrent work.

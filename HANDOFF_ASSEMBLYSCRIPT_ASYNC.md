# AssemblyScript Async Backend Handoff

## Objective

Complete the AssemblyScript backend: executable async exports/imports, future and
stream endpoints, error-context support, typed ownership, cancellation, tests,
and both AssemblyScript runtimes (`incremental` and `minimal`).

## How To Start

Workspace `D:\wit-bindgen`, branch `feat/assemblyscript-backend`.

`crates/c/README.md` and `crates/c/src/lib.rs` are permanently dirty with
unrelated concurrent work. **Never stage `crates/c/*`.** Everything else should
be clean.

### Toolchain

Node is required for every AssemblyScript test — both runtime *and* codegen,
because codegen verification shells out to `asc --noEmit`.

Node is installed under vfox but **is not on PATH by default**. Prefix commands:

```bash
export PATH="$HOME/.vfox/cache/nodejs/v-25.8.2/nodejs-25.8.2:$PATH"
```

If `node`/`npm`/`asc` are missing: the vfox SDK directories can end up
present-but-empty, in which case `vfox install nodejs@<v>` reports "already
installed" and does nothing. Recover with `vfox uninstall nodejs@<v>`, then
install, then `npm i -g assemblyscript`. (`26.2.0` cannot currently be removed —
`opencode2.exe` inside it is locked by the running process — hence `25.8.2`.)

### Verified green baseline at `0afabc43`

Re-run these before changing anything; they should all pass.

```bash
export PATH="$HOME/.vfox/cache/nodejs/v-25.8.2/nodejs-25.8.2:$PATH"

cargo fmt --check
cargo test -p wit-bindgen-assemblyscript                 # 19 passed
cargo clippy -p wit-bindgen-assemblyscript -p wit-bindgen-test --all-targets -- -D warnings
git diff --check

# AssemblyScript codegen (type-checks generated output with asc)
cargo run test --languages assemblyscript tests/codegen --artifacts target/a-cg   # 212 passed

# Cross-backend codegen — the shared core ABI is touched by this work
cargo run test --languages rust,moonbit,c tests/codegen --artifacts target/a-cb \
  --wasi-sdk-path /c/wasi-sdk                                                     # 1378 passed

# Runtime suite, BOTH AssemblyScript runtimes (195 + 111 passed each)
cargo run test --languages rust,assemblyscript tests/runtime \
  --artifacts target/a-inc --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript tests/runtime \
  --artifacts target/a-min --rust-wit-bindgen-path ./crates/guest-rust
```

## What Landed (and why it matters)

Two commits, both after an adversarial review of everything since `7942297b`.

### `298a745e` — ownership, cancellation, deallocation

- `DropHandle` re-wrapped an already-lifted operand (`new Item(new Item(h)).drop()`),
  passing a heap address to `[resource-drop]`. `abi::deallocate` lifts before
  emitting `DropHandle`; the backend now drops the lifted operand directly.
- `GuestDeallocateVariant` discarded every payload block and emitted **nothing**,
  so `option`/`result`/`variant` leaked their active payload and never dropped
  owned handles inside it. Now emits the tag `switch`.
- `GuestDeallocateList`/`Map` discarded the element block and freed using the
  element *count* as the byte size. Now emit the per-element loop and free
  `len * elem_size` with the element alignment.
- Cancellation could issue two completions: `Scheduler.resume` called
  `task.cancel` eagerly on `EVENT_CANCEL` then resumed the task anyway. Now it
  records the cancel, lets the task clean up, and forces `EXIT`; the generated
  finish helper issues whichever of `task.return`/`task.cancel` the task did not
  and traps on an exit that is neither.
- Finish helper freed the task before clearing context-0 (`release` before
  `complete`); reversed.
- `threadYield()` discarded the `[thread-yield]` cancellation result; returns
  `bool` now.
- `ErrorContext.debugMessage` decoded before its null check.
- `subtaskHandle`/`waitableCount` used arithmetic shifts on unsigned fields.
- Removed the dead managed `AsyncSubtask` class.
- Removed stale `should_fail_verify` markers for `named-fixed-length-list.wit-async`
  in `crates/test/src/rust.rs` and `moonbit.rs` that `ef8c43e3` left behind,
  which had the codegen suite failing "should have failed, but passed".

### `0afabc43` — the runtime was never initialized

The generated `asconfig.json` set `exportStart: "_start"`, which suppresses the
wasm `(start)` section and exports `_start` for an embedder to call. **Nothing
calls it in a component**, so the TLSF heap was never initialized and the *first*
managed allocation aborted.

This is why every fixture that ever passed used only scalars, and it was the real
cause of the long-standing "managed async import results trap under incremental"
gap. Three recorded hypotheses were refuted by experiment:

- *subtask self-free corrupting the TLSF free list* — reproduces with the free
  removed entirely;
- *GC/ownership sequencing after the async call* — reproduces on a bare
  `String.UTF16.decodeUnsafe` at the top of an exported function, before any
  async call;
- *UTF-8/UTF-16 mismatch* — the bytes are correct UTF-16LE with the correct
  length.

Also fixed the leak this uncovered: `finish()` ran `cleanup(false)`, deallocating
nothing. Lifting *copies* lists/strings but *transfers* owned handles, so the
success path now runs a lists-only deallocation and the discard path keeps
lists-and-own.

`tests/runtime/return-string/{runner.ts,test.rs}` is the first AssemblyScript
fixture to round-trip a managed value.

---

# Remaining Work

Ordered by value. Item 1 is a prerequisite for trusting anything else.

## 1. Fixtures replace generated code instead of exercising it — HIGHEST VALUE

**This is the defect that let every bug above survive a "fully validated" handoff.
Fix it before adding features.**

`crates/test/src/assemblyscript.rs` copies each fixture over the generated file
named by its `//@ path`. Every async `test.ts` sets `path = "exports/<iface>.ts"`,
so the fixture **replaces the entire generated interface file** — task base,
`__exp_*`, `__callback_*`, `__finish___exp_*`, and the `@external` endpoint
declarations. Verified byte-identical: the copied artifact equals the fixture
source.

Consequences:

- The fixtures test hand-written replicas of the generator's output, not the
  generator. `tests/runtime/simple-future/test.ts` and `simple-stream/test.ts`
  hand-declare their own `[future-read-*]`/`[stream-read-*]` intrinsics,
  bypassing `emit_future_stream_helpers` entirely.
- Every `runner.ts` passes `//@ args = '--async=-run'`, lowering the export under
  test synchronously. **No generated AssemblyScript async export is executed by
  any runtime test.**
- Demonstrated live: changing the shared runtime's cancellation protocol broke
  `cancel-import` while every generated-code unit test stayed green, because the
  fixture carries its own copy of the finish helper.

By contrast Rust fixtures `include!(env!("BINDINGS"))` and implement only the
trait.

**Target shape:** fixtures implement only the task subclass and consume the
generated export glue; `//@ path` points at a user file the generated code
imports, rather than over the top of it; `--async=-run` is dropped.

**Acceptance:** deliberately breaking `emit_async_export_support` or the finish
helper must fail a runtime test.

**Note:** `tests/runtime/simple-future/test.ts:77` has
`rawExportDropFutureFuture0Read` stubbed as `return -1;`, and
`simple-import-params-results/test.ts` tasks never suspend (every `resume`
finishes immediately). Both hide generated-code paths.

## 2. AssemblyScript coverage is 7 of 60 runtime directories

Missing fixtures are **silently skipped, never failed**. `crates/test/src/lib.rs`
discovers components by scanning for files whose names start with a world name;
unmatched files hit a `log::debug!` and produce no entry. The only hard failures
are "no runner at all" / "no test at all", which `runner.rs`/`test.rs` satisfy.
CI runs `--languages rust,c,assemblyscript tests/runtime` and reports green
regardless.

Have `.ts`: `cancel-import`, `numbers`, `return-string`, `simple-future`,
`simple-import-params-results`, `simple-stream`, `simple-yield`.

The other 53 have none. Now that managed values work, the high-value ones are
newly reachable and roughly ordered:

- **Strings/lists (newly unblocked by `0afabc43`):** `strings`, `strings-simple`,
  `strings-alias`, `lists`, `lists-alias`, `list-in-variant`, `many-arguments`,
  `options`, `results`, `variants`, `records`, `map`, `fixed-length-lists`.
- **Resources (exercises the open bugs in §3):** `resources`, `resource-borrow`,
  `resource-import-and-export`, `resource_aggregates`, `resource_alias`,
  `resource_alias_redux`, `resource_borrow_in_record`, `resource_floats`,
  `resource_with_lists`.
- **Async depth:** `pending-import`, `simple-pending-import`,
  `yield-loop-receives-events`, `future-cancel-read`, `future-cancel-write`,
  `future-cancel-write-then-read`, `future-close-*`, `future-write-then-read-*`,
  `incomplete-writes`, `stream-to-futures-stream`, `simple-stream-payload`,
  `ping-pong`, `simple-call-import`.
- **Misc:** `flavorful`, `common-types`, `symbol-conflicts`, `unused-types`,
  `versions`, `package-with-version`, `gated-features`.

Skip the language-specific dirs (`c`, `cpp`, `rust`, `moonbit`, `demo`, `rust-*`).

Also: `tests/runtime/cancel-import/runner.ts` implements 2 of the 5 scenarios in
`runner.rs`. Missing are cancelling in the *started* state with backpressure, and
the two races — cancellation against a completed status code, and against a
STARTING→STARTED transition. Those races are exactly what `*Subtask.update`/
`cancel` state handling exists for.

Consider making a missing fixture a hard failure once coverage is reasonable, or
the same blind spot returns.

## 3. Exported resources are not routed through the instance table

For a resource the guest exports that is passed out through an imported async
function, the generator resolves the `use`d resource to an **imported** wrapper
class and lowers it as a raw `.handle`, instead of acquiring a handle with
`__Item_take` and releasing it with `__Item_drop_instance`. The exported instance
table is emitted but never called on this path.

Reproduce with the world in
`exported_resource_through_async_import_is_not_yet_routed_through_the_instance_table`
(`crates/assemblyscript/src/lib.rs`), which asserts the *current* behaviour and
is written to fail once this is fixed — replace it with a positive assertion then.

Generated today:

```ts
export function run(value: i_test$bindings$api.Item): RunSubtask {
  const subtask = new RunSubtask(value.handle);   // raw handle, wrong identity
```

Separately: for a plain async **export** taking `own<item>`, no `DropHandle` is
emitted at all, so the table entry leaks. Owned-handle cleanup only runs on the
async *import* path.

## 4. No post-return support

The AssemblyScript backend implements no post-return at all — there is no
`post_return` anywhere in `crates/assemblyscript/src/`. `GuestDeallocate*` is
therefore reachable **only** through async-import cleanup, so synchronous exports
never release owned parameters. Every sync export taking a string, list, or owned
resource leaks it.

This also means the `GuestDeallocateVariant`/`List`/`Map` fixes in `298a745e` are
currently exercised only via async imports.

## 5. Error contexts are never dropped

Verified against generated output for `tests/codegen/error-context.wit`: the
export lifts an owned incoming error-context and never releases it.

```ts
const v0 = bar(new async_.ErrorContext(<i32>a0), new async_.ErrorContext(<i32>a1), ...);
```

No `.drop()` anywhere. `ErrorContextLower` emits `{op}.handle` with no ownership
bookkeeping, so a user calling `.drop()` after passing one to an import
double-drops. Rust's `ErrorContext` has a `Drop` impl; AS has no finalizers, so
this must be driven by explicit cleanup.

Note `error_context_uses_typed_wrapper_and_drop` still does not assert a drop —
its name is aspirational. Fix the emission, then make the test honest, then add a
runtime fixture (there is no `tests/runtime/error-context/`; the earlier
experiment was deleted).

## 6. Sync import list/string buffers leak

`crates/assemblyscript/src/lib.rs:2832` and `:2878` both do `let _ = realloc;` and
unconditionally allocate a copy. For a **sync** import the core ABI sets
`Realloc::None` precisely because the callee only borrows and the caller retains
ownership — but nothing frees the buffer after the call. One leaked buffer per
list argument per sync import call.

`StringLower` already does the right thing (zero-copy when `realloc.is_none()`);
`ListCanonLower`/`ListLower` should match, or emit the free after `CallWasm`.

## 7. Smaller items and hazards

- `ffi.ts` `cabi_realloc` ignores `_align`, relying on AS TLSF returning 16-byte
  alignment. True for every WIT alignment today; add an assert or a comment.
- `ffi.ts` lift helpers do `new Uint8Array(<i32>len)` with a `u32` host-supplied
  length; a length ≥ 2^31 becomes negative. Bounds-check.
- `crates/core/src/abi.rs` fixed-length-list deallocation is **fully unrolled**
  (`size` copies) rather than using a block + `IterBasePointer` like every
  sibling case. Correct, but a code-size hazard in *every* backend for large
  fixed-length lists. Also note this changed sync post-return behaviour for all
  backends (previously fixed-length list contents were never freed).
- `async.ts` `WaitableSet.wait()`/`poll()` and `Scheduler.wait` are unsafe under
  the callback ABI (a callback-ABI task must return `callbackWait(set)`, never
  block) and `WaitableSet` is managed while tasks are `@unmanaged`. Currently
  unreferenced by generated code. Either delete or document as stackful-only.
- `async.ts` single-instance return areas (`memory.data(8)`, `memory.data(16)`)
  are not re-entrancy safe; `ffi.ts`'s own header comment warns about this.
- Typed future helpers are export-side only; streams have no typed payload API at
  all. `emit_typed_future_helpers` starts its `next_endpoint` cursor at 0 while
  indexing into the whole function's endpoint list, which is wrong for nested
  cases like `future<future<T>>` (unverified; the `cleanup()` path compensates
  correctly, these helpers do not).
- `crates/assemblyscript/src/lib.rs:17-20` module doc is a garbled leftover.
- Unconditional codegen `panic!`s for tuple arity > 16 and flags count > 64, with
  no tests.

## Design Constraints (do not regress)

**Unmanaged export tasks.** Generated/user async export task hierarchies are
`@unmanaged`. Persist only scalars, raw pointers, handles, booleans, integer
state — never managed strings, arrays, records, or class references across a
yield. AssemblyScript virtual dispatch reads a managed runtime type header, so
calling an overridden method through an unmanaged base is unsafe: the scheduler
captures the concrete method reference in `AsyncTask.resumeIndex` at
`Scheduler.start` and resumes via `call_indirect`. The fixtures subclass the
generated base and override `resume`, so this is exercised.

**Async-import subtasks.** Generated async imports return a per-call `@unmanaged`
concrete `*Subtask` owning status/state/handle, a result-area pointer, lowered
scalar params, and lifecycle flags:

```ts
const subtask = imports.someAsync(...);
const value = subtask.finish(status);  // STATUS_RETURNED only
// or subtask.dispose(status) to cancel / discard the result
```

**Endpoint DropHandle.** `Instruction::DropHandle` carries only a type, not an
endpoint occurrence, so the backend tracks order with `next_endpoint`. Keep the
duplicate-type regression test; parameter occurrences must precede result ones.

**Reentrancy.** `Scheduler.start` traps when context-0 is occupied. A
save/restore stack is *not* correct — callback arguments carry no task identity,
so either restored pointer could route the next callback to the wrong task.

**asconfig.** Do not reintroduce `exportStart`. See `0afabc43`.

## Review Hygiene

Three parallel adversarial reviewers produced this list. Several of their
highest-confidence claims were **wrong** and cost real time; verify before acting:

- "The `abi.rs` `ErrorContext` change breaks MoonBit codegen (`todo!()`)" —
  MoonBit passes `tests/codegen/error-context.wit` today.
- "`resume` is never dispatched, so every async export traps" — the fixtures
  override `resume` and pass.
- "The subtask self-free corrupts the TLSF free list" — refuted by experiment.

Equally, my own intermediate conclusions ("decode works standalone", "debug
builds pass") were artifacts of a probe sitting in a branch that never executed
at that iteration count. **Confirm which branch runs before concluding anything
from a passing test.**

## Useful Files

- Generator: `crates/assemblyscript/src/lib.rs`
- Embedded runtime: `crates/assemblyscript/src/async/async.ts`, `async.rs`
- FFI: `crates/assemblyscript/src/ffi/ffi.ts`
- Endpoints: `emit_future_stream_helpers`, `emit_typed_future_helpers`,
  `emit_async_import_subtask`
- Tasks: `emit_async_task_base`, `emit_async_export_support`
- Harness: `crates/test/src/assemblyscript.rs`, `crates/test/src/lib.rs`
- Canonical references: `crates/guest-rust/src/rt/async_support/`,
  `crates/moonbit/src/async_support.rs`, `crates/c/src/lib.rs`

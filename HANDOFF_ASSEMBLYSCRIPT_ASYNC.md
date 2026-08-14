# AssemblyScript Backend Handoff

## Objective

Complete the AssemblyScript backend: executable async exports/imports, future
and stream endpoints, error-context support, typed ownership, cancellation,
tests, and both AssemblyScript runtimes (`incremental` and `minimal`).

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

### Line endings

`crates/assemblyscript/src/async/async.ts` and `ffi/ffi.ts` are committed with
LF and are asserted against verbatim by unit tests (`include_str!`). Editing
them with a tool that rewrites newlines turns the file CRLF and breaks
`scheduler_rejects_reentrant_tasks` with a confusing message. Convert back
before running tests.

### Verified green baseline

Re-run these before changing anything; they should all pass.

```bash
export PATH="$HOME/.vfox/cache/nodejs/v-25.8.2/nodejs-25.8.2:$PATH"

cargo fmt --check
cargo test -p wit-bindgen-assemblyscript                 # 32 passed
cargo clippy -p wit-bindgen-assemblyscript -p wit-bindgen-test --all-targets -- -D warnings
git diff --check

# AssemblyScript codegen (type-checks generated output with asc)
cargo run test --languages assemblyscript tests/codegen --artifacts target/a-cg   # 212 passed

# Cross-backend codegen — only needed when the shared core ABI is touched
cargo run test --languages rust,moonbit,c tests/codegen --artifacts target/a-cb \
  --wasi-sdk-path /c/wasi-sdk                                                     # 1378 passed

# Runtime suite, BOTH AssemblyScript runtimes
cargo run test --languages rust,assemblyscript tests/runtime \
  --artifacts target/a-inc --rust-wit-bindgen-path ./crates/guest-rust
WIT_BINDGEN_AS_RUNTIME=minimal cargo run test --languages rust,assemblyscript tests/runtime \
  --artifacts target/a-min --rust-wit-bindgen-path ./crates/guest-rust
```

### Writing a fixture

Generate the bindings for the world first and read the stub it produces —
guessing the mangled paths and names wastes time:

```bash
cargo run -q -- assemblyscript tests/runtime/<dir>/test.wit --world <world> --out-dir /tmp/gen
ls /tmp/gen/stubs /tmp/gen/imports
cat /tmp/gen/stubs/*.ts        # the signatures your fixture must match
```

`//@ path` points at one of those `stubs/*.ts`. The basename mangling is
`ident::iface_basename`: `:` `/` `@` become `$`, and `.` `-` become `_`, so
`test:many-arguments/to-test` is `test$many_arguments$to_test` and
`my:inline/foo@0.0.0` is `my$inline$foo$0_0_0`. World-level exports go to
`stubs/world.ts`.

The import side of the same WIT is `/tmp/gen/imports/<basename>.ts` — generate
the `runner` world separately to see it. Resource functions are
`constructorX` / `methodXGetA` / `staticXAdd` on both sides.

Note that AssemblyScript accepts no `--generate-unused-types`: it always emits
every type an interface declares, so a fixture copied from Rust must drop that
`//@ args` line.

### Localizing a wasm trap

Release builds carry no name section, so a failing runtime test gives you
`<wasm function 42>` and nothing else. Every guest-side failure is a bare
`unreachable`, so the only way to tell two of them apart is to make one of them
*not* trap:

```ts
function assert(condition: bool): void {
  if (!condition) { while (true) {} }   // hangs instead of trapping
}
```

A hang (the run takes its full timeout) means that site fired; a fast failure
means something else did. Bisect by converting sites back to `unreachable()`
one group at a time. The same trick works on the generator by editing what it
emits — that is how item 1 below was narrowed to the generated task base's
`resume`, and how a DROPPED-versus-COMPLETED stream event was identified.

Two caveats. The trap may be in the *other* component: pair your fixture
against the Rust one to find out which side is wrong. And `assert` in
AssemblyScript routes through `ffi.abort`, so an allocator or bounds failure
looks identical to a failed assertion — patch `abort` itself to loop if you
need to tell those apart.

---

# What Landed

Eight commits on top of `1318ec2b`.

### `5495ebfd` — generated glue split from the user stub

The defect that let everything below survive a "fully validated" handoff.
Fixtures were copied over `exports/<iface>.ts`, replacing the whole generated
interface file — task bases, `__exp_*`, `__callback_*`, `__finish_*`, and the
`@external` endpoint declarations — with hand-written replicas, and every
runner passed `--async=-run`. No generated AssemblyScript async export was ever
executed, and breaking the generator left every test green.

Generated export files now import their user half from `stubs/<basename>.ts`
(`stubs/world.ts` for world-level exports). `exports/<basename>.ts` and
`world.ts` hold generated glue only and are always regenerated; `--ignore-stub`
preserves the stub file alone. Every `.ts` fixture is rewritten against the new
layout, and no runner passes `--async=-run`.

**Acceptance, re-checked:** deleting the `task.return` call from
`emit_async_export_support` now fails `simple-yield` and `simple-future`.

### `e2de0977` — export results, export params, borrowed import buffers

- No post-return existed at all. `cabi_post_*` is now emitted for exports whose
  result needs deallocation and has a return pointer.
- Synchronous exports never released the buffers and handles the caller lowered
  into their memory. Lifting *copies*, so every string, list, error-context and
  owned handle parameter leaked. Released after the user call.
- `ListCanonLower`/`ListLower` ignored `realloc`, which is `Realloc::None` for a
  synchronous import precisely because the callee only borrows. Numeric lists
  are now passed in place; non-canonical lists free their lowered buffer and any
  nested list or string after the call.

### `3f4b9108` — async export parameters, FFI hardening

The task owns its parameters for its whole life, so the task base carries the
raw arguments as `__arg<i>` fields and the finish helper releases them on both
the returned and cancelled paths. Plus: `cabi_realloc` asserts the alignment it
can actually satisfy, typed-array lift helpers bounds-check the host-supplied
`u32` length, the stackful-only helpers in `async.ts` are documented as such,
and the two hard codegen limits (tuple arity 16, flags 64) have tests.

### `24015cd3` — exported resources, and local-name shadowing

**Exported resources had never run.** `__X_take` returned its own table index as
if it were a component-model handle, so the host rejected the first constructor
call with "unknown handle index". The table now speaks the canonical protocol:
`[resource-new]` to mint a handle, `[resource-rep]` to resolve an owned one,
`[resource-drop]` to release it. A `borrow<x>` lifted inside the component that
owns `x` arrives as the *rep*, not a handle — conflating the two was the second
half of the failure.

**Generated locals shadowed parameters.** They were `v0`, `v1`, ..., exactly the
shape a WIT parameter can have; `resource_aggregates` has `foo(..., v1: v1, v2:
v2, ...)`, where `const v1 = <second argument>` shadowed the parameter and every
later read took the wrong value. Arguments were silently swapped with no
diagnostic. Generated locals now use a `__`-prefixed namespace.

Also: unrolled fixed-length lists replayed one block source per element into a
single scope, which `asc` rejects. Each replay now gets its own scope.

### `3a568eab` — the export-parameter ownership contract

Releasing *every* owned parameter was too aggressive. The `leaf-toplevel`
component of `resource-import-and-export` is `return a`: the wrapper dropped the
incoming handle and then returned it. Ordering cannot fix that, so the contract
is explicit — see `FunctionBindgen::export_param_cleanup`.

### `d899c110` — exported resource method names

`resources` exports two resources that both define `get-a`. Stripping the
`[method]`/`[static]`/`[constructor]` qualification collapsed them into one
`getA` and `asc` rejected the file. Resource-associated stubs keep their
qualification: `constructorX`, `methodXGetA`, `staticXAdd`.

### `01562172` — typed stream payload helpers

Futures had typed payload helpers on the task base; streams had only the raw
endpoint imports. An exported `stream<T>` now gets `startStream<i>Read` /
`finishStream<i>Read` and `startStream<i>Write` / `finishStream<i>Write`, backed
by a growable element buffer on the task, so the payload is exchanged as an
ordinary AssemblyScript list. This commit also rewrote this document.

### `01070c74` — code-review follow-ups

From a two-axis review of everything above. Removed inert scaffolding in
`tuples_wider_than_sixteen_are_rejected`; made `gen_export_function` save and
restore `emitting_stub` instead of clobbering it; sharpened
`simple-call-import/runner.ts`, which had become byte-identical to
`simple-pending-import/runner.ts` and no longer distinguished the two cases;
folded the third copy of the `list<T>` typed-array mapping into
`list_type_ref`; named the `AsyncParamCleanup` pair. Documented `// @@file:` in
`crates/test/README.md`.

---

# Remaining Work

## 1. Two concurrently-suspended export tasks trap

Reproducible, localized, root cause **not** identified. With two async export
tasks suspended at the same time in one AssemblyScript component, the next
resume dispatches into the generated task base's `unreachable()` — the
placeholder `resume` that a subclass is supposed to override.

Established by experiment (each of these was a separate run):

- It is *not* `Scheduler.start`'s reentrancy guard: making that guard loop
  instead of trap does not hang.
- It is *not* `Scheduler.resume`'s `ptr == 0` guard: same test.
- Context-0 identity is fine: a task asserting `contextGet() == changetype<usize>(this)`
  on both of its resumes passes.
- Making the generated base `resume()` loop instead of trap turns the failure
  into a hang; making only the finish helper's `else` branch loop does not.
  So the base `resume` is what is reached.

Which means `resumeIndex` — captured in `Scheduler.start` from `task.resume` and
dispatched with `call_indirect` — is right on the first resume and wrong on a
later one, only when a second task is alive. Suspect the interaction between
AssemblyScript's method-reference lowering for `@unmanaged` classes and
`Scheduler.start`'s generic instantiation.

To reproduce: `tests/runtime/simple-import-params-results/runner.ts` currently
drives its five imports sequentially; issuing two of them concurrently and
waiting on one waitable set fails. `simple-stream-payload/runner.ts` was dropped
for what is probably the same reason (`runner.rs | test.ts` passes, so the
AssemblyScript *callee* is fine).

Until this is understood, an AssemblyScript component cannot serve two
concurrent async export calls.

## 2. Runtime coverage is 34 of 60 directories

Was 7. `crates/test/src/lib.rs` still *silently skips* a language with no
fixture — only "no runner at all" / "no test at all" is a hard failure — so the
suite stays green regardless. Consider making a missing AssemblyScript fixture a
hard failure once the async directories below are covered.

Covered: `cancel-import`, `fixed-length-lists`, `list-in-variant`,
`lists-alias`, `many-arguments`, `map`, `numbers`, `options`,
`package-with-version`, `records`, `resource-borrow`,
`resource-import-and-export`, `resource_aggregates`, `resource_alias`,
`resource_alias_redux`, `resource_borrow_in_record`, `resource_with_lists`,
`resources`, `results`, `return-string`, `simple-call-import`, `simple-future`,
`simple-import-params-results`, `simple-pending-import`, `simple-stream`,
`simple-stream-payload` (callee only), `simple-yield`, `strings`,
`strings-alias`, `strings-simple`, `symbol-conflicts`, `unused-types`,
`variants`, `versions`.

Not covered, roughly by value:

- **Async depth:** `pending-import`, `yield-loop-receives-events`,
  `future-cancel-read`, `future-cancel-write`, `future-cancel-write-then-read`,
  `future-close-after-coming-back`, `future-close-then-receive-read`,
  `future-closes-with-error`, `future-write-then-read-comes-back`,
  `future-write-then-read-remote`, `incomplete-writes`,
  `stream-to-futures-stream`, `ping-pong`.
- **Misc:** `flavorful`, `common-types` (needs three fixtures; has no Rust
  counterpart, so it would be AssemblyScript-only), `gated-features`,
  `resource_floats`.
- `lists` is deliberately skipped: its runner asserts that the callee's
  `allocated-bytes` is unchanged across every call, and AssemblyScript exposes
  no allocated-byte counter. Adding it with a constant `0` would make the
  runner's leak assertions vacuous. Closing this properly means a counter in
  `cabi_realloc`, which costs every user.

Skip the language-specific dirs (`c`, `cpp`, `rust`, `moonbit`, `demo`,
`rust-*`).

## 3. Ownership gaps that remain

- A `list<T>` nested inside a **record or variant parameter** of a synchronous
  import is still not released. `ListLower` only queues its cleanup at the top
  level, because a nested list's buffer and length are loop-locals that no
  longer exist after the call. See the comment at the `realloc.is_none()` guard.
- Lowering a `borrow<T>` of an **exported** resource mints a fresh *owned*
  handle with `__X_take`. That is worse than a leak: the callee would drop the
  handle, running the destructor while the guest still holds the instance. It is
  the mirror of the lift-side bug `24015cd3` fixed. No world reaches it today —
  WIT forbids returning a borrow, and an import taking one resolves to the
  *imported* view of the resource — so the branch is noted at the `HandleLower`
  site rather than fixed with an instance-to-rep map that nothing would use.
- `abi.rs` unrolls fixed-length-list deallocation rather than using a block plus
  `IterBasePointer`. Correct, but a code-size hazard in every backend. Folding
  it into a loop needs a new core instruction that every backend must implement.

## 4. Error contexts have no runtime coverage

The previous handoff asked to "add a runtime fixture (there is no
`tests/runtime/error-context/`; the earlier experiment was deleted)". Still
absent. The unit test is honest now — it asserts that the export does *not* drop
a parameter, because `round-trip: func(context: error-context) -> error-context`
hands it straight back — but nothing exercises an error context end to end, and
under the ownership contract a user who forgets `drop()` leaks with no
diagnostic. Creating the directory means writing the WIT and a Rust counterpart
too.

## 5. Smaller items

- `async.ts` single-instance return areas (`memory.data(8)`, `memory.data(16)`)
  are documented as non-re-entrant but not made safe.
- Typed future/stream payload helpers are export-side only. The import side
  still hands out raw endpoint helpers.
- `crates/test/src/assemblyscript.rs` splits a fixture on `// @@file: <path>`
  markers so one file can supply the several stubs a multi-interface world
  needs (documented in `crates/test/README.md`). Rust solves the same problem
  with `externs = ['./other.rs']`; the two conventions have not been unified.
- Review debt, all judgement calls: `sync_export_param_cleanup` and
  `async_export_param_cleanup` share a six-step shape that could be one
  function; `render_interface_file`/`render_stub_file` share their import
  preamble; the `kind: &str` in {"imports","exports","world"} is switched on in
  three places and wants an enum; `FunctionBindgen` now configures four fields
  by assignment after `new`; `lib.rs` is at ~5600 lines and, like
  `crates/rust`, could split its `Bindgen` impl into its own module.
- Fixture debt: 25 `.ts` fixtures each redeclare `function assert`, which the
  Rust and C fixtures get from `std`/`<assert.h>` — a shared AssemblyScript test
  prelude has no obvious home, since each fixture is copied into its own
  bindings directory. New fixtures also mix short import aliases (`I`, `E`)
  with the mangled `i_<iface>` form the older ones use.

---

## Design Constraints (do not regress)

**Generated locals are `__`-prefixed.** No WIT identifier can produce a leading
underscore, so `__v0`/`__d0`/`__p0` cannot shadow a parameter. Keep
`generated_locals_cannot_shadow_a_parameter`.

**Export-parameter ownership.** The generator releases only what it made itself
responsible for: list and string buffers (lifting copied them, and the user
never sees a pointer), and owned handles to resources this world *exports* (the
user gets the class instance and cannot name the handle). Imported-resource
wrappers, error contexts, and future/stream endpoints all carry an explicit
`drop()` and are the user's; dropping them behind the user's back is a
use-after-free, not a leak, and passing them back out is legal.

**Exported resources use the canonical intrinsics.** `[resource-new]` to mint,
`[resource-rep]` to resolve an owned handle, `[resource-drop]` to release. A
`borrow<x>` lifted inside the owning component *is* the rep.

**Unmanaged export tasks.** Generated/user async export task hierarchies are
`@unmanaged`. Persist only scalars, raw pointers, handles, booleans, integer
state — never managed strings, arrays, records, or class references across a
yield. AssemblyScript virtual dispatch reads a managed runtime type header, so
the scheduler captures the concrete method reference in `AsyncTask.resumeIndex`
at `Scheduler.start` and resumes via `call_indirect`.

**Async-import subtasks.** Generated async imports return a per-call
`@unmanaged` concrete `*Subtask`:

```ts
const subtask = imports.someAsync(...);
const value = subtask.finish(status);  // STATUS_RETURNED only
// or subtask.dispose(status) to cancel / discard the result
```

**Endpoint DropHandle.** `Instruction::DropHandle` carries only a type, not an
endpoint occurrence, so the backend tracks order with `next_endpoint`. Keep the
duplicate-type regression test; parameter occurrences must precede result ones.

**Reentrancy.** `Scheduler.start` traps when context-0 is occupied. A
save/restore stack is *not* correct — callback arguments carry no task identity.

**asconfig.** Do not reintroduce `exportStart`. See `0afabc43`.

---

## Review Hygiene

Claims inherited from the previous handoff that turned out to be **wrong**, with
the evidence:

- *"Exported resources passed through an async import should be routed through
  the instance table with `__Item_take`."* They should not. A world that exports
  an interface and imports something using that interface's resource has two
  distinct resource types; `wit-bindgen-rust` splits the same world into
  `test::bindings::api::Item` and `exports::test::bindings::api::Item`. The raw
  `.handle` lowering was already right.
- *"`emit_typed_future_helpers` starts its `next_endpoint` cursor at 0 while
  indexing into the whole function's endpoint list, which is wrong for nested
  cases."* Checked against generated output for
  `future<future<u32>>` + `stream<string>` + `-> future<u32>`: the indices match
  `emit_future_stream_helper` and the result type-checks.
- *"Error contexts must be dropped by the export wrapper."* They must not — see
  the ownership contract above. `round-trip: func(context: error-context) ->
  error-context` is exactly the shape that breaks.

And from the handoff before that, still worth repeating: several
highest-confidence review claims were wrong and cost real time. Confirm which
branch actually runs before concluding anything from a passing test.

---

## Useful Files

- Generator: `crates/assemblyscript/src/lib.rs`
- Embedded runtime: `crates/assemblyscript/src/async/async.ts`, `async.rs`
- FFI: `crates/assemblyscript/src/ffi/ffi.ts`
- Endpoints: `emit_future_stream_helpers`, `emit_typed_future_helpers`,
  `emit_typed_stream_helpers`, `emit_async_import_subtask`
- Tasks: `emit_async_task_base`, `emit_async_export_support`
- Ownership: `sync_export_param_cleanup`, `async_export_param_cleanup`,
  `free_borrowed_list`, `emit_post_return`
- Harness: `crates/test/src/assemblyscript.rs`, `crates/test/src/lib.rs`
- Canonical references: `crates/guest-rust/src/rt/async_support/`,
  `crates/moonbit/src/async_support.rs`, `crates/c/src/lib.rs`

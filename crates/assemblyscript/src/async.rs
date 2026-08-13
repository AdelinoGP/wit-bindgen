//! Async runtime template embedded into every generated output.
//!
//! The contents of `src/async/async.ts` are checked into this repository and
//! shipped verbatim as `async.ts` in the generated bindings. It contains the
//! shared stackless runtime: waitable-set wrappers, event/status constants,
//! subtask cancellation, backpressure, context helpers, and the explicit task
//! scheduler. Per-function `task.return` intrinsics are emitted next to their
//! exports. Future/stream payload endpoint operations are not generated yet.

pub const ASYNC_TS: &str = include_str!("./async/async.ts");

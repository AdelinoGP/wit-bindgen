//! FFI helper template embedded into every generated output.
//!
//! The contents of `src/ffi/ffi.ts` are checked into this repository and shipped
//! verbatim. The generator concatenates a per-world `__IMPORT_RETURN_AREA`
//! declaration onto the end before writing the final `ffi.ts` to the output.

pub const FFI_TS: &str = include_str!("./ffi/ffi.ts");

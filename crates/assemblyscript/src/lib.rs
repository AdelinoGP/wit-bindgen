//! AssemblyScript guest-language bindings generator for `wit-bindgen`.
//!
//! Structurally mirrors `crates/moonbit` (UTF-16 strings, finalizer-less GC,
//! explicit resource drop) but emits per-interface `.ts` files under
//! `imports/` and `exports/` subdirectories of the output, à la
//! `crates/go`'s per-package layout.
//!
//! # Memory-safety discipline
//!
//! AssemblyScript GC marks named locals as roots. The generated lowering code
//! always materializes pointer-source AS objects (strings, arrays) into named
//! locals **before** taking raw pointers, and keeps them live through the
//! enclosing wasm-import call. No explicit `__pin` / `__unpin` is emitted.
//!
//! # Output layout
//!
//! Each exported interface is emitted as two files. `exports/<basename>.ts`
//! (or `world.ts` for world-level exports) holds generated glue only — types,
//! wasm-export wrappers, async task bases, callbacks, and endpoint
//! declarations — and is always regenerated. `stubs/<basename>.ts` holds the
//! user's implementation: the exported resource classes and the entrypoint
//! each export dispatches to. `--ignore-stub` preserves the latter only.
//!
//! # Async
//!
//! Async callback bindings use an explicit stackless `AsyncTask` API because
//! AssemblyScript has no native async/await. `future<T>` and `stream<T>`
//! remain opaque handles, with raw endpoint helpers exposing canonical payload
//! pointers for explicit lifting and lowering.

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::mem;
use wit_bindgen_core::abi::{self, AbiVariant, Bindgen, Bitcast, Instruction, LiftLower, WasmType};
use wit_bindgen_core::wit_parser::{
    Alignment, ArchitectureSize, Docs, Enum, Flags, FlagsRepr, Function, FutureIntrinsic, Handle,
    InterfaceId, LiftLowerAbi, Mangling, ManglingAndAbi, Record, Resolve, Result_, SizeAlign,
    StreamIntrinsic, Tuple, Type, TypeDefKind, TypeId, TypeOwner, Variant, WasmExport,
    WasmExportKind, WasmImport, WorldId, WorldKey,
};
use wit_bindgen_core::{
    AsyncFilterSet, Direction, Files, InterfaceGenerator as CoreInterfaceGenerator, WorldGenerator,
};

mod r#async;
mod ffi;
mod ident;

const VERSION: &str = env!("CARGO_PKG_VERSION");
// =============================================================================
// Opts
// =============================================================================

#[derive(Default, Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Runtime {
    #[default]
    Incremental,
    Minimal,
}

impl Runtime {
    fn as_asc_name(self) -> &'static str {
        match self {
            Runtime::Incremental => "incremental",
            Runtime::Minimal => "minimal",
        }
    }
}

#[derive(Default, Debug, Clone)]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
pub struct Opts {
    /// AssemblyScript runtime variant. `incremental` is the default AS runtime
    /// (TLSF + automatic GC). `minimal` is smaller and requires manual
    /// `__collect()` but works for short-lived components.
    #[cfg_attr(feature = "clap", arg(long, value_enum, default_value = "incremental"))]
    pub runtime: Runtime,

    /// Skip overwriting files the user is expected to edit: `exports/*.ts` and
    /// `asconfig.json`. Other generated files are always overwritten.
    #[cfg_attr(feature = "clap", arg(long, default_value_t = false))]
    pub ignore_stub: bool,

    /// Select functions to bind with the canonical async callback ABI.
    #[cfg_attr(feature = "clap", clap(flatten))]
    pub async_: AsyncFilterSet,
}

impl Opts {
    pub fn build(&self) -> Box<dyn WorldGenerator> {
        Box::new(AssemblyScript {
            opts: self.clone(),
            ..AssemblyScript::default()
        })
    }
}

// =============================================================================
// Per-interface fragments and the WorldGenerator
// =============================================================================

/// A growing source fragment for one interface plus the foreign-interface
/// imports it depends on.
#[derive(Default)]
struct InterfaceFragment {
    /// Generated source body (types + function declarations).
    src: String,
    /// User-implementation source body, emitted to `stubs/<basename>.ts`.
    ///
    /// Everything the user is expected to edit lives here — exported resource
    /// classes and the entrypoint each exported function dispatches to. The
    /// generated glue in `src` is therefore always safe to overwrite.
    stub_src: String,
    /// Value names (user functions) the glue imports from the stub file.
    stub_values: BTreeSet<String>,
    /// Type names (exported resource classes) defined in the stub file. The
    /// glue imports these *and* re-exports them, so sibling files keep
    /// referring to them as `e_<basename>.<Type>`.
    stub_types: BTreeSet<String>,
    /// Set of "<basename>"s of sibling interface files this fragment imports
    /// types or call helpers from. Each becomes an `import * as i_<basename>`
    /// line at the top of the emitted file.
    imports_imports: BTreeSet<String>,
    imports_exports: BTreeSet<String>,
    needs_async: bool,
}

/// Stage where exported wasm functions land while we build the world.
#[derive(Default)]
struct ExportEntry {
    /// Canonical WIT wasm-export name (target of the post-compile rename).
    wasm_name: String,
    /// AssemblyScript identifier the wrapper is defined under (source of the
    /// rename; what asc actually emits as the wasm export name).
    as_name: String,
    /// File basename under `exports/` containing the wrapper definition.
    /// `bindings.ts` re-exports the identifier from this file.
    /// Empty string for world-level exports (kept in `world.ts`).
    basename: String,
    /// (Currently unused — historical scratch.)
    #[allow(dead_code, reason = "historical scratch, kept for reference")]
    body: String,
}

#[derive(Default)]
pub struct AssemblyScript {
    opts: Opts,

    /// Pre-computed size + alignment for every WIT type in the world.
    sizes: SizeAlign,

    /// Max size and alignment for the return area of any import in the world.
    /// Drives the size of `__IMPORT_RETURN_AREA` in `ffi.ts`.
    import_return_area_size: usize,
    import_return_area_align: usize,

    /// Interfaces we've already issued a file for, keyed by interface id.
    /// One map per direction because `Direction` doesn't implement `Hash`.
    import_basenames: HashMap<InterfaceId, String>,
    export_basenames: HashMap<InterfaceId, String>,

    /// Per-interface fragments awaiting flush.
    import_interfaces: BTreeMap<String, InterfaceFragment>,
    export_interfaces: BTreeMap<String, InterfaceFragment>,

    /// World-level imports / exports (functions and types not behind an interface).
    world_imports: InterfaceFragment,
    world_exports: InterfaceFragment,

    /// Camel-cased names of freestanding functions exported by the world.
    /// Used to avoid source-level collisions with same-named imports.
    world_export_names: BTreeSet<String>,

    /// Wasm-export entries collected during export processing. Keyed by canonical
    /// wasm export name; value contains the AS function + body to inline into
    /// `bindings.ts`.
    exports: Vec<ExportEntry>,

    /// Counters for unique handle counters per exported resource (one per
    /// exported resource type id).
    exported_resources: Vec<TypeId>,
    needs_async: bool,
}

impl WorldGenerator for AssemblyScript {
    fn preprocess(&mut self, resolve: &Resolve, _world: WorldId) -> Result<()> {
        self.sizes.fill(resolve);
        Ok(())
    }

    fn import_interface(
        &mut self,
        resolve: &Resolve,
        key: &WorldKey,
        id: InterfaceId,
        files: &mut Files,
    ) -> Result<()> {
        let basename = self.register_interface(resolve, key, id, Direction::Import);
        let mut r#gen = InterfaceGenerator::new(
            self,
            resolve,
            Some(id),
            Some(key.clone()),
            Direction::Import,
        );
        r#gen.types_inline(id);
        for (_, func) in resolve.interfaces[id].functions.iter() {
            r#gen.gen_import_function(func);
        }
        let fragment = r#gen.finish();
        let _ = files; // files are flushed in finish()
        self.import_interfaces.insert(basename, fragment);
        Ok(())
    }

    fn import_funcs(
        &mut self,
        resolve: &Resolve,
        world: WorldId,
        funcs: &[(&str, &Function)],
        _files: &mut Files,
    ) {
        self.world_export_names = resolve.worlds[world]
            .exports
            .iter()
            .filter_map(|(key, item)| match (key, item) {
                (WorldKey::Name(name), wit_bindgen_core::wit_parser::WorldItem::Function(_)) => {
                    Some(ident::value_name(name))
                }
                _ => None,
            })
            .collect();
        let mut r#gen = InterfaceGenerator::new(self, resolve, None, None, Direction::Import);
        for (_, func) in funcs {
            r#gen.gen_import_function(func);
        }
        let fragment = r#gen.finish();
        self.world_imports.concat(fragment);
    }

    fn import_types(
        &mut self,
        resolve: &Resolve,
        _world: WorldId,
        types: &[(&str, TypeId)],
        _files: &mut Files,
    ) {
        let mut r#gen = InterfaceGenerator::new(self, resolve, None, None, Direction::Import);
        for (name, id) in types {
            r#gen.define_type(name, *id);
        }
        let fragment = r#gen.finish();
        self.world_imports.concat(fragment);
    }

    fn export_interface(
        &mut self,
        resolve: &Resolve,
        key: &WorldKey,
        id: InterfaceId,
        files: &mut Files,
    ) -> Result<()> {
        let basename = self.register_interface(resolve, key, id, Direction::Export);
        let mut r#gen = InterfaceGenerator::new(
            self,
            resolve,
            Some(id),
            Some(key.clone()),
            Direction::Export,
        );
        r#gen.types_inline(id);
        for (_, func) in resolve.interfaces[id].functions.iter() {
            r#gen.gen_export_function(func);
        }
        let fragment = r#gen.finish();
        let _ = files;
        self.export_interfaces.insert(basename, fragment);
        Ok(())
    }

    fn export_funcs(
        &mut self,
        resolve: &Resolve,
        _world: WorldId,
        funcs: &[(&str, &Function)],
        _files: &mut Files,
    ) -> Result<()> {
        let mut r#gen = InterfaceGenerator::new(self, resolve, None, None, Direction::Export);
        for (_, func) in funcs {
            r#gen.gen_export_function(func);
        }
        let fragment = r#gen.finish();
        self.world_exports.concat(fragment);
        Ok(())
    }

    fn finish(&mut self, _resolve: &Resolve, _world: WorldId, files: &mut Files) -> Result<()> {
        self.opts.async_.ensure_all_used()?;
        // Emit imports/<basename>.ts files.
        let import_interfaces = mem::take(&mut self.import_interfaces);
        for (basename, frag) in &import_interfaces {
            let path = format!("imports/{basename}.ts");
            let body = render_interface_file(frag, "imports", basename);
            files.push(&path, body.as_bytes());
        }

        // Emit exports/<basename>.ts (generated glue, always overwritten) and
        // stubs/<basename>.ts (the user's implementation, skipped under
        // --ignore-stub).
        let export_interfaces = mem::take(&mut self.export_interfaces);
        for (basename, frag) in &export_interfaces {
            let path = format!("exports/{basename}.ts");
            let body = render_interface_file(frag, "exports", basename);
            files.push(&path, body.as_bytes());
            if !self.opts.ignore_stub {
                let path = format!("stubs/{basename}.ts");
                let body = render_stub_file(frag, "exports", basename);
                files.push(&path, body.as_bytes());
            }
        }

        // World-level imports/exports → world.ts
        if !self.world_imports.src.is_empty() || !self.world_exports.src.is_empty() {
            let mut combined = InterfaceFragment::default();
            combined
                .imports_imports
                .extend(self.world_imports.imports_imports.iter().cloned());
            combined
                .imports_imports
                .extend(self.world_exports.imports_imports.iter().cloned());
            combined
                .imports_exports
                .extend(self.world_imports.imports_exports.iter().cloned());
            combined
                .imports_exports
                .extend(self.world_exports.imports_exports.iter().cloned());
            combined.needs_async = self.world_imports.needs_async || self.world_exports.needs_async;
            combined.src.push_str(&self.world_imports.src);
            combined.src.push('\n');
            combined.src.push_str(&self.world_exports.src);
            combined.stub_src.push_str(&self.world_exports.stub_src);
            combined
                .stub_values
                .extend(self.world_exports.stub_values.iter().cloned());
            combined
                .stub_types
                .extend(self.world_exports.stub_types.iter().cloned());
            let body = render_interface_file(&combined, "world", "");
            files.push("world.ts", body.as_bytes());
            if !self.opts.ignore_stub && !combined.stub_src.is_empty() {
                let body = render_stub_file(&combined, "world", "");
                files.push("stubs/world.ts", body.as_bytes());
            }
        }

        // bindings.ts: the asc entry. Wasm-export wrappers inlined; dispatches
        // into user-edited exports/*.ts via `e_<basename>.<func>(...)`.
        let bindings = self.render_bindings_ts(&import_interfaces, &export_interfaces);
        files.push("bindings.ts", bindings.as_bytes());

        // Sidecar for the post-compile wasm export-section renamer.
        let renames = self.render_exports_rename_json();
        files.push("wit_bindgen_exports.json", renames.as_bytes());

        // ffi.ts: helpers + __IMPORT_RETURN_AREA sized to this world.
        let ffi_ts = self.render_ffi_ts();
        files.push("ffi.ts", ffi_ts.as_bytes());

        if self.needs_async {
            files.push("async.ts", r#async::ASYNC_TS.as_bytes());
        }

        // asconfig.json: target config consumed by `asc --target release`.
        if !self.opts.ignore_stub {
            let asc = self.render_asconfig_json();
            files.push("asconfig.json", asc.as_bytes());
        }
        Ok(())
    }
}

impl AssemblyScript {
    fn register_interface(
        &mut self,
        resolve: &Resolve,
        key: &WorldKey,
        id: InterfaceId,
        dir: Direction,
    ) -> String {
        let name = match key {
            WorldKey::Name(n) => n.clone(),
            WorldKey::Interface(iface_id) => {
                let iface = &resolve.interfaces[*iface_id];
                let pkg = iface
                    .package
                    .map(|p| {
                        let pname = &resolve.packages[p].name;
                        let ver = pname
                            .version
                            .as_ref()
                            .map(|v| format!("@{v}"))
                            .unwrap_or_default();
                        format!(
                            "{}:{}/{}{ver}",
                            pname.namespace,
                            pname.name,
                            iface.name.as_deref().unwrap_or("")
                        )
                    })
                    .unwrap_or_else(|| iface.name.clone().unwrap_or_else(|| "iface".into()));
                pkg
            }
        };
        let basename = ident::iface_basename(&name);
        match dir {
            Direction::Import => {
                self.import_basenames.insert(id, basename.clone());
            }
            Direction::Export => {
                self.export_basenames.insert(id, basename.clone());
            }
        }
        basename
    }

    fn render_bindings_ts(
        &self,
        imports: &BTreeMap<String, InterfaceFragment>,
        exports: &BTreeMap<String, InterfaceFragment>,
    ) -> String {
        let mut s = String::new();
        writeln!(s, "// Generated by wit-bindgen {VERSION}. DO NOT EDIT.").unwrap();
        writeln!(s, "//").unwrap();
        writeln!(
            s,
            "// Entry point for `asc`. Wasm-export wrappers below dispatch into"
        )
        .unwrap();
        writeln!(
            s,
            "// user-edited files under `exports/`. The wrappers are emitted with"
        )
        .unwrap();
        writeln!(
            s,
            "// AS-valid identifier names; a post-compile step renames the wasm"
        )
        .unwrap();
        writeln!(s, "// export section to match the canonical WIT names (see").unwrap();
        writeln!(s, "// wit_bindgen_exports.json for the rename map).").unwrap();
        writeln!(s, "//").unwrap();
        writeln!(s, "import * as ffi from \"./ffi\";").unwrap();
        if self.needs_async {
            writeln!(s, "import * as async_ from \"./async\";").unwrap();
        }
        // Re-export cabi_realloc so the host can allocate guest memory.
        writeln!(s, "export {{ cabi_realloc }} from \"./ffi\";").unwrap();
        writeln!(s).unwrap();

        for basename in imports.keys() {
            writeln!(s, "import * as i_{basename} from \"./imports/{basename}\";").unwrap();
        }
        for basename in exports.keys() {
            writeln!(s, "import * as e_{basename} from \"./exports/{basename}\";").unwrap();
        }
        if !self.world_imports.src.is_empty() || !self.world_exports.src.is_empty() {
            writeln!(s, "import * as world from \"./world\";").unwrap();
        }
        writeln!(s).unwrap();

        let _ = imports;

        // Plain re-export each wrapper by AS identifier. asc emits each as a
        // wasm export under that identifier; the post-compile rewriter renames
        // the export-section entries to the canonical WIT names listed in
        // wit_bindgen_exports.json.
        let mut by_basename: BTreeMap<&str, Vec<&ExportEntry>> = BTreeMap::new();
        for entry in &self.exports {
            by_basename
                .entry(entry.basename.as_str())
                .or_default()
                .push(entry);
        }
        for (basename, entries) in &by_basename {
            writeln!(s).unwrap();
            let names: Vec<&str> = entries.iter().map(|e| e.as_name.as_str()).collect();
            let from = if basename.is_empty() {
                "./world".to_string()
            } else {
                format!("./exports/{basename}")
            };
            writeln!(s, "export {{ {} }} from \"{from}\";", names.join(", ")).unwrap();
        }

        s
    }

    /// Sidecar JSON read by the build step (or test harness) to rewrite the
    /// wasm export section from AS identifiers → canonical WIT export names.
    fn render_exports_rename_json(&self) -> String {
        let mut s = String::from("{\n");
        let mut first = true;
        for entry in &self.exports {
            if !first {
                s.push_str(",\n");
            }
            first = false;
            // Escape backslash and double-quote in both keys and values just
            // in case of exotic WIT identifiers.
            let from = entry.as_name.replace('\\', "\\\\").replace('"', "\\\"");
            let to = entry.wasm_name.replace('\\', "\\\\").replace('"', "\\\"");
            s.push_str(&format!("  \"{from}\": \"{to}\""));
        }
        s.push_str("\n}\n");
        s
    }

    fn render_ffi_ts(&self) -> String {
        let mut s = String::new();
        s.push_str(ffi::FFI_TS);
        s.push('\n');
        writeln!(
            s,
            "// ---------------------------------------------------------------------------"
        )
        .unwrap();
        writeln!(
            s,
            "// World-specific return area. Sized at generation time to the maximum"
        )
        .unwrap();
        writeln!(
            s,
            "// canonical-ABI return area required by any single import in this world."
        )
        .unwrap();
        writeln!(
            s,
            "// ---------------------------------------------------------------------------"
        )
        .unwrap();
        writeln!(s).unwrap();
        let size = self.import_return_area_size.max(8);
        let _align = self.import_return_area_align.max(4);
        writeln!(
            s,
            "@global export const __IMPORT_RETURN_AREA: usize = changetype<usize>("
        )
        .unwrap();
        writeln!(s, "  memory.data({size})").unwrap();
        writeln!(s, ");").unwrap();
        s
    }

    fn render_asconfig_json(&self) -> String {
        format!(
            // No `exportStart`: that would suppress the wasm `(start)` section
            // and export `_start` for an embedder to call instead. Nothing calls
            // it in a component, so the AssemblyScript runtime — including the
            // TLSF heap — would stay uninitialized and the first managed
            // allocation would abort. Emitting a real start section makes the
            // component model run initialization at instantiation.
            "{{\n  \"targets\": {{\n    \"release\": {{\n      \"outFile\": \"core.wasm\",\n      \"runtime\": \"{runtime}\",\n      \"exportRuntime\": true,\n      \"optimizeLevel\": 3,\n      \"shrinkLevel\": 0,\n      \"converge\": false,\n      \"noAssert\": false,\n      \"use\": [\"abort=ffi/abort\"]\n    }},\n    \"debug\": {{\n      \"outFile\": \"core.wasm\",\n      \"runtime\": \"{runtime}\",\n      \"exportRuntime\": true,\n      \"debug\": true,\n      \"use\": [\"abort=ffi/abort\"]\n    }}\n  }}\n}}\n",
            runtime = self.opts.runtime.as_asc_name()
        )
    }
}

fn render_interface_file(frag: &InterfaceFragment, kind: &str, basename: &str) -> String {
    // `kind` is "imports" or "exports" → file lives one level down, prefix is
    // "../". For "world" the file lives at the root, prefix is "./".
    let prefix = if kind == "world" { "./" } else { "../" };
    let mut s = String::new();
    writeln!(s, "// Generated by wit-bindgen {VERSION}. DO NOT EDIT.").unwrap();
    writeln!(s).unwrap();
    writeln!(s, "import * as ffi from \"{prefix}ffi\";").unwrap();
    if frag.needs_async {
        writeln!(s, "import * as async_ from \"{prefix}async\";").unwrap();
    }
    for other in &frag.imports_imports {
        writeln!(s, "import * as i_{other} from \"{prefix}imports/{other}\";").unwrap();
    }
    for other in &frag.imports_exports {
        writeln!(s, "import * as e_{other} from \"{prefix}exports/{other}\";").unwrap();
    }
    // The user-implementation half of this interface. Resource classes defined
    // there are re-exported so sibling files keep seeing them on this module.
    let names: Vec<&str> = frag
        .stub_types
        .iter()
        .chain(frag.stub_values.iter())
        .map(String::as_str)
        .collect();
    if !names.is_empty() {
        writeln!(
            s,
            "import {{ {} }} from \"{prefix}{}\";",
            names.join(", "),
            stub_module(kind, basename)
        )
        .unwrap();
        if !frag.stub_types.is_empty() {
            let types: Vec<&str> = frag.stub_types.iter().map(String::as_str).collect();
            writeln!(s, "export {{ {} }};", types.join(", ")).unwrap();
        }
    }
    writeln!(s).unwrap();
    s.push_str(&frag.src);
    s
}

/// Path (relative to the output root, minus extension) of the stub file paired
/// with an export glue file.
fn stub_module(kind: &str, basename: &str) -> String {
    if kind == "world" {
        "stubs/world".to_string()
    } else {
        format!("stubs/{basename}")
    }
}

/// The user-implementation file paired with `exports/<basename>.ts` (or
/// `world.ts`). Never regenerated once written when `--ignore-stub` is set.
fn render_stub_file(frag: &InterfaceFragment, kind: &str, basename: &str) -> String {
    let mut s = String::new();
    writeln!(
        s,
        "// Generated by wit-bindgen {VERSION} as a starting point. Edit freely:"
    )
    .unwrap();
    writeln!(
        s,
        "// this file holds your implementation, and `--ignore-stub` preserves it."
    )
    .unwrap();
    writeln!(s).unwrap();
    writeln!(s, "import * as ffi from \"../ffi\";").unwrap();
    if frag.needs_async {
        writeln!(s, "import * as async_ from \"../async\";").unwrap();
    }
    for other in &frag.imports_imports {
        writeln!(s, "import * as i_{other} from \"../imports/{other}\";").unwrap();
    }
    for other in &frag.imports_exports {
        writeln!(s, "import * as e_{other} from \"../exports/{other}\";").unwrap();
    }
    if kind == "world" {
        writeln!(s, "import * as world from \"../world\";").unwrap();
    } else if !frag.imports_exports.contains(basename) {
        writeln!(
            s,
            "import * as e_{basename} from \"../exports/{basename}\";"
        )
        .unwrap();
    }
    writeln!(s).unwrap();
    s.push_str(&frag.stub_src);
    s
}

impl InterfaceFragment {
    fn concat(&mut self, other: InterfaceFragment) {
        self.src.push_str(&other.src);
        self.stub_src.push_str(&other.stub_src);
        self.stub_values.extend(other.stub_values);
        self.stub_types.extend(other.stub_types);
        self.imports_imports.extend(other.imports_imports);
        self.imports_exports.extend(other.imports_exports);
        self.needs_async |= other.needs_async;
    }
}

// =============================================================================
// InterfaceGenerator: emits types and function declarations
// =============================================================================

struct InterfaceGenerator<'a> {
    world_gen: &'a mut AssemblyScript,
    resolve: &'a Resolve,
    interface: Option<InterfaceId>,
    /// World key for the current interface, used for canonical wasm
    /// import/export-module name resolution. `None` for world-level
    /// freestanding funcs/types.
    interface_key: Option<WorldKey>,
    direction: Direction,
    src: String,
    /// User-implementation source for `stubs/<basename>.ts`.
    stub_src: String,
    /// True while writing into `stub_src`. Named types owned by this interface
    /// live in the glue file, so references from the stub file must be
    /// qualified with the glue namespace.
    emitting_stub: bool,
    stub_values: BTreeSet<String>,
    stub_types: BTreeSet<String>,
    imports_imports: BTreeSet<String>,
    imports_exports: BTreeSet<String>,
    needs_async: bool,
}

impl<'a> InterfaceGenerator<'a> {
    fn new(
        world_gen: &'a mut AssemblyScript,
        resolve: &'a Resolve,
        interface: Option<InterfaceId>,
        interface_key: Option<WorldKey>,
        direction: Direction,
    ) -> Self {
        Self {
            world_gen,
            resolve,
            interface,
            interface_key,
            direction,
            src: String::new(),
            stub_src: String::new(),
            emitting_stub: false,
            stub_values: BTreeSet::new(),
            stub_types: BTreeSet::new(),
            imports_imports: BTreeSet::new(),
            imports_exports: BTreeSet::new(),
            needs_async: false,
        }
    }

    /// Namespace alias under which the *glue* file for this interface is
    /// imported by its stub file.
    fn glue_ns(&self) -> String {
        match self
            .interface
            .and_then(|id| self.world_gen.export_basenames.get(&id))
        {
            Some(basename) => format!("e_{basename}"),
            None => "world".to_string(),
        }
    }

    /// Run `f` with output redirected into the user-implementation file.
    fn with_stub(&mut self, f: impl FnOnce(&mut Self)) {
        let outer = mem::take(&mut self.src);
        let was_stub = mem::replace(&mut self.emitting_stub, true);
        f(self);
        self.emitting_stub = was_stub;
        let stub = mem::replace(&mut self.src, outer);
        self.stub_src.push_str(&stub);
    }

    fn types_inline(&mut self, iface: InterfaceId) {
        let types: Vec<(String, TypeId)> = self.resolve.interfaces[iface]
            .types
            .iter()
            .map(|(n, id)| (n.clone(), *id))
            .collect();
        for (name, id) in &types {
            self.define_type(name, *id);
        }
    }

    fn define_type(&mut self, name: &str, id: TypeId) {
        wit_bindgen_core::define_type(self, name, id);
    }

    fn finish(self) -> InterfaceFragment {
        InterfaceFragment {
            src: self.src,
            stub_src: self.stub_src,
            stub_values: self.stub_values,
            stub_types: self.stub_types,
            imports_imports: self.imports_imports,
            imports_exports: self.imports_exports,
            needs_async: self.needs_async,
        }
    }

    /// Emit doc comment if non-empty.
    fn docs(&mut self, docs: &Docs) {
        if let Some(text) = &docs.contents {
            for line in text.lines() {
                writeln!(self.src, "/// {line}").unwrap();
            }
        }
    }

    /// AssemblyScript type expression for a WIT type, handling foreign-interface
    /// references via the import-collection sets.
    fn type_ref(&mut self, ty: &Type) -> String {
        match ty {
            Type::Bool => "bool".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::S8 => "i8".into(),
            Type::S16 => "i16".into(),
            Type::S32 => "i32".into(),
            Type::S64 => "i64".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Char => "i32".into(),
            Type::String => "string".into(),
            Type::ErrorContext => {
                self.needs_async = true;
                self.world_gen.needs_async = true;
                "async_.ErrorContext".into()
            }
            Type::Id(id) => self.type_id_ref(*id),
        }
    }

    fn type_id_ref(&mut self, id: TypeId) -> String {
        let ty = &self.resolve.types[id];
        match &ty.kind {
            TypeDefKind::Handle(Handle::Own(resource_id))
            | TypeDefKind::Handle(Handle::Borrow(resource_id)) => self.named_type_ref(*resource_id),
            TypeDefKind::Type(t) => self.type_ref(t),
            TypeDefKind::List(elem) => {
                let inner = self.type_ref(elem);
                match elem {
                    Type::U8 => "Uint8Array".into(),
                    Type::S8 => "Int8Array".into(),
                    Type::U16 => "Uint16Array".into(),
                    Type::S16 => "Int16Array".into(),
                    Type::U32 => "Uint32Array".into(),
                    Type::S32 => "Int32Array".into(),
                    Type::U64 => "Uint64Array".into(),
                    Type::S64 => "Int64Array".into(),
                    Type::F32 => "Float32Array".into(),
                    Type::F64 => "Float64Array".into(),
                    _ => format!("Array<{inner}>"),
                }
            }
            TypeDefKind::Option(inner) => {
                let it = self.type_ref(inner);
                format!("ffi.Option<{it}>")
            }
            TypeDefKind::Result(r) => {
                let ok =
                    r.ok.map(|t| self.type_ref(&t))
                        .unwrap_or_else(|| "i32".into());
                let err = r
                    .err
                    .map(|t| self.type_ref(&t))
                    .unwrap_or_else(|| "i32".into());
                format!("ffi.Result<{ok}, {err}>")
            }
            TypeDefKind::Variant(_)
            | TypeDefKind::Record(_)
            | TypeDefKind::Enum(_)
            | TypeDefKind::Flags(_)
            | TypeDefKind::Resource => self.named_type_ref(id),
            TypeDefKind::Tuple(t) => tuple_ref(self, t),
            TypeDefKind::Future(_) | TypeDefKind::Stream(_) => "i32".into(),
            TypeDefKind::FixedLengthList(t, _size) => {
                let inner = self.type_ref(t);
                format!("StaticArray<{inner}>")
            }
            TypeDefKind::Map(k, v) => {
                let kt = self.type_ref(k);
                let vt = self.type_ref(v);
                format!("Map<{kt}, {vt}>")
            }
            TypeDefKind::Unknown => "i32 /* unknown */".into(),
        }
    }

    /// Resolve a TypeId to its AS type name, recording any cross-interface
    /// `import` that needs to appear at the top of the emitted file. World-
    /// level type aliases (created via `use foo.{bar}`) follow the alias chain
    /// so we resolve to the interface that actually defines the type.
    fn named_type_ref(&mut self, id: TypeId) -> String {
        let canonical_id = wit_bindgen_core::dealias(self.resolve, id);
        let ty = &self.resolve.types[canonical_id];
        let raw = ty.name.clone().unwrap_or_default();
        let local = ident::type_name(&raw);
        match ty.owner {
            TypeOwner::Interface(other) if Some(other) != self.interface => {
                let (basename, alias_prefix) = self.foreign_basename(other);
                format!("{alias_prefix}{basename}.{local}")
            }
            // Own type. Defined in the glue file, except exported resource
            // classes, which the user implements in the stub file itself.
            _ if self.emitting_stub && !self.is_own_exported_resource(canonical_id) => {
                format!("{}.{local}", self.glue_ns())
            }
            _ => local,
        }
    }

    /// True for a resource this world exports whose class body lives in this
    /// interface's stub file.
    fn is_own_exported_resource(&self, id: TypeId) -> bool {
        self.world_gen.exported_resources.contains(&id)
    }

    /// Look up the basename of a foreign interface. Prefers import-side
    /// classification; falls back to export-side.
    fn foreign_basename(&mut self, other: InterfaceId) -> (String, &'static str) {
        if let Some(bn) = self.world_gen.import_basenames.get(&other) {
            let bn = bn.clone();
            self.imports_imports.insert(bn.clone());
            return (bn, "i_");
        }
        if let Some(bn) = self.world_gen.export_basenames.get(&other) {
            let bn = bn.clone();
            self.imports_exports.insert(bn.clone());
            return (bn, "e_");
        }
        // Not yet registered (probably a forward reference). Synthesize and
        // remember; the actual file will be emitted later.
        let iface = &self.resolve.interfaces[other];
        let raw = iface.name.clone().unwrap_or_else(|| "iface".to_string());
        let bn = ident::iface_basename(&raw);
        self.world_gen.import_basenames.insert(other, bn.clone());
        self.imports_imports.insert(bn.clone());
        (bn, "i_")
    }

    /// Lower a WIT name into the AS field-name idiom (camelCase + reserved-suffix).
    fn field_ident(name: &str) -> String {
        ident::value_name(name)
    }

    fn func_ident(name: &str) -> String {
        ident::value_name(name)
    }

    // ---- function generation ----------------------------------------------

    fn gen_import_function(&mut self, func: &Function) {
        // `[method]Foo.bar`, `[static]Foo.bar`, `[constructor]Foo` flow through
        // the same code path; we just unwrap their resource ID for naming.
        let func_local = Self::func_ident(&func.name);
        let wrapper_name = if self.interface.is_none()
            && self.world_gen.world_export_names.contains(&func_local)
        {
            format!("__import_{func_local}")
        } else {
            func_local.clone()
        };
        let is_async = self.world_gen.opts.async_.is_async(
            self.resolve,
            self.interface_key.as_ref(),
            func,
            true,
        );
        if is_async {
            self.needs_async = true;
            self.world_gen.needs_async = true;
        }
        self.emit_future_stream_helpers(func, false);
        let (wasm_module, raw_extern_name) = self.resolve.wasm_import_name(
            ManglingAndAbi::Legacy(if is_async {
                LiftLowerAbi::AsyncCallback
            } else {
                LiftLowerAbi::Sync
            }),
            WasmImport::Func {
                interface: self.interface_key.as_ref(),
                func,
            },
        );
        let mangled_extern = format!("__ext_{}", sanitize_extern_local(&func_local));

        // Signature: emit @external `declare` + a friendly wrapper.
        let variant = if is_async {
            AbiVariant::GuestImportAsync
        } else {
            AbiVariant::GuestImport
        };
        let sig = self.func_signature(func, variant);

        self.docs(&func.docs);
        writeln!(
            self.src,
            "@external(\"{wasm_module}\", \"{raw_extern_name}\")"
        )
        .unwrap();
        let wasm_sig = self.resolve.wasm_signature(variant, func);
        let params_ext: Vec<String> = wasm_sig
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("a{i}: {}", wasm_type_name(*t)))
            .collect();
        let ret_ext = if wasm_sig.results.is_empty() {
            "void".to_string()
        } else if wasm_sig.results.len() == 1 {
            wasm_type_name(wasm_sig.results[0]).to_string()
        } else {
            // Multi-value wasm returns are routed through the return-area pointer
            // (the canonical ABI uses an out-pointer arg in this case). We
            // conservatively type this as `void` since the wrapper handles lifting.
            "void".to_string()
        };
        writeln!(
            self.src,
            "declare function {mangled_extern}({}): {ret_ext};",
            params_ext.join(", ")
        )
        .unwrap();
        writeln!(self.src).unwrap();

        if is_async {
            self.emit_async_import_subtask(func, &func_local, &mangled_extern, &wasm_sig);
        }

        // Friendly wrapper: lifts args from AS values into wasm types, calls
        // the import, lifts results back.
        let wrapper = if is_async {
            sig.async_import_signature(&wrapper_name, &func_local)
        } else {
            sig.wrapper_signature(&wrapper_name)
        };
        writeln!(self.src, "export function {wrapper} {{").unwrap();
        if is_async {
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                variant,
                LiftLower::LowerArgsLiftResults,
                mangled_extern.clone(),
            );
            let mut wasm_params = Vec::new();
            if wasm_sig.indirect_params {
                let layout = bindgen
                    .iface_gen
                    .world_gen
                    .sizes
                    .record(func.params.iter().map(|p| &p.ty));
                let ptr = "__params";
                bindgen.push_line(&format!(
                    "const {ptr} = ffi.cabi_realloc(0, 0, {}, {});",
                    layout.align.align_wasm32(),
                    layout.size.size_wasm32()
                ));
                for ((offset, ty), param) in bindgen
                    .iface_gen
                    .world_gen
                    .sizes
                    .field_offsets(func.params.iter().map(|p| &p.ty))
                    .into_iter()
                    .zip(func.params.iter())
                {
                    abi::lower_to_memory(
                        bindgen.iface_gen.resolve,
                        &mut bindgen,
                        format!("{ptr} + {}", offset.size_wasm32()),
                        ident::value_name(&param.name),
                        ty,
                    );
                }
                wasm_params.push(ptr.to_string());
            } else {
                for param in &func.params {
                    wasm_params.extend(abi::lower_flat(
                        bindgen.iface_gen.resolve,
                        &mut bindgen,
                        ident::value_name(&param.name),
                        &param.ty,
                    ));
                }
            }
            let subtask_type = format!("{}Subtask", ident::type_name(&func_local));
            bindgen.push_line(&format!(
                "const subtask = new {subtask_type}({});",
                wasm_params.join(", ")
            ));
            bindgen.push_line("subtask.start();");
            bindgen.push_line("return subtask;");
            for line in bindgen.into_body().lines() {
                writeln!(self.src, "  {line}").unwrap();
            }
            writeln!(self.src, "}}").unwrap();
            writeln!(self.src).unwrap();
            return;
        }

        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestImport,
            LiftLower::LowerArgsLiftResults,
            mangled_extern.clone(),
        );
        abi::call(
            bindgen.iface_gen.resolve,
            AbiVariant::GuestImport,
            LiftLower::LowerArgsLiftResults,
            func,
            &mut bindgen,
            false,
        );
        let body = bindgen.into_body();
        for line in body.lines() {
            writeln!(self.src, "  {line}").unwrap();
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn emit_async_import_subtask(
        &mut self,
        func: &Function,
        func_local: &str,
        mangled_extern: &str,
        wasm_sig: &wit_bindgen_core::abi::WasmSignature,
    ) {
        let subtask_type = format!("{}Subtask", ident::type_name(func_local));
        let result_count = usize::from(func.result.is_some());
        let param_types = &wasm_sig.params[..wasm_sig.params.len() - result_count];
        let fields = param_types
            .iter()
            .enumerate()
            .map(|(i, ty)| format!("a{i}: {}", wasm_type_name(*ty)))
            .collect::<Vec<_>>();

        writeln!(self.src, "@unmanaged\nexport class {subtask_type} {{").unwrap();
        writeln!(self.src, "  status: i32 = async_.STATUS_STARTING;").unwrap();
        writeln!(self.src, "  state: i32 = async_.STATUS_STARTING;").unwrap();
        writeln!(self.src, "  handle: i32 = 0;").unwrap();
        writeln!(self.src, "  result: usize = 0;").unwrap();
        writeln!(self.src, "  private started: bool = false;").unwrap();
        writeln!(self.src, "  private cancellationRequested: bool = false;").unwrap();
        writeln!(self.src, "  private paramsReleased: bool = false;").unwrap();
        writeln!(self.src, "  private finished: bool = false;").unwrap();
        for field in &fields {
            writeln!(self.src, "  {field};").unwrap();
        }
        if !fields.is_empty() {
            writeln!(self.src, "  constructor({}) {{", fields.join(", ")).unwrap();
            for i in 0..fields.len() {
                writeln!(self.src, "    this.a{i} = a{i};").unwrap();
            }
            writeln!(self.src, "  }}").unwrap();
        }

        writeln!(self.src, "  start(): i32 {{").unwrap();
        writeln!(self.src, "    if (this.started) unreachable();").unwrap();
        writeln!(self.src, "    this.started = true;").unwrap();
        if let Some(result) = &func.result {
            let layout = self.world_gen.sizes.record([result]);
            writeln!(
                self.src,
                "    this.result = ffi.cabi_realloc(0, 0, {}, {});",
                layout.align.align_wasm32(),
                layout.size.size_wasm32()
            )
            .unwrap();
        }
        let mut args = (0..fields.len())
            .map(|i| format!("this.a{i}"))
            .collect::<Vec<_>>();
        if func.result.is_some() {
            args.push("this.result".into());
        }
        writeln!(
            self.src,
            "    this.update({mangled_extern}({}));",
            args.join(", ")
        )
        .unwrap();
        writeln!(self.src, "    return this.status;").unwrap();
        writeln!(self.src, "  }}").unwrap();
        writeln!(self.src, "  update(status: i32): void {{").unwrap();
        writeln!(self.src, "    if (!this.started) unreachable();").unwrap();
        writeln!(self.src, "    if (this.finished) unreachable();").unwrap();
        writeln!(self.src, "    this.status = status;").unwrap();
        writeln!(self.src, "    this.state = async_.subtaskState(status);").unwrap();
        writeln!(self.src, "    const handle = async_.subtaskHandle(status);").unwrap();
        writeln!(self.src, "    if (handle != 0) this.handle = handle;").unwrap();
        writeln!(
            self.src,
            "    if (this.state == async_.STATUS_STARTED_CANCELLED) {{"
        )
        .unwrap();
        writeln!(self.src, "      this.releaseParams(true);").unwrap();
        writeln!(
            self.src,
            "    }} else if (this.state == async_.STATUS_STARTED ||"
        )
        .unwrap();
        writeln!(
            self.src,
            "               this.state == async_.STATUS_RETURNED ||"
        )
        .unwrap();
        writeln!(
            self.src,
            "               this.state == async_.STATUS_RETURNED_CANCELLED) {{"
        )
        .unwrap();
        writeln!(self.src, "      this.releaseParams(false);").unwrap();
        writeln!(self.src, "    }}").unwrap();
        writeln!(self.src, "  }}").unwrap();

        writeln!(self.src, "  cancel(): i32 {{").unwrap();
        writeln!(
            self.src,
            "    if (!this.started || this.finished || this.cancellationRequested || this.handle == 0) unreachable();"
        )
        .unwrap();
        writeln!(self.src, "    this.cancellationRequested = true;").unwrap();
        writeln!(
            self.src,
            "    const status = async_.subtaskCancel(this.handle);"
        )
        .unwrap();
        writeln!(self.src, "    this.update(status);").unwrap();
        writeln!(self.src, "    return status;").unwrap();
        writeln!(self.src, "  }}").unwrap();

        writeln!(self.src, "  private releaseParams(own: bool): void {{").unwrap();
        writeln!(self.src, "    if (this.paramsReleased) return;").unwrap();
        writeln!(self.src, "    this.paramsReleased = true;").unwrap();
        let param_operands = (0..fields.len())
            .map(|i| format!("this.a{i}"))
            .collect::<Vec<_>>();
        let param_types = func.params.iter().map(|p| p.ty).collect::<Vec<_>>();
        writeln!(self.src, "    if (own) {{").unwrap();
        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestImportAsync,
            LiftLower::LowerArgsLiftResults,
            String::new(),
        );
        abi::deallocate_lists_and_own_in_types(
            bindgen.iface_gen.resolve,
            &param_types,
            &param_operands,
            wasm_sig.indirect_params,
            &mut bindgen,
        );
        for line in bindgen.into_body().lines() {
            writeln!(self.src, "      {line}").unwrap();
        }
        writeln!(self.src, "    }} else {{").unwrap();
        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestImportAsync,
            LiftLower::LowerArgsLiftResults,
            String::new(),
        );
        abi::deallocate_lists_in_types(
            bindgen.iface_gen.resolve,
            &param_types,
            &param_operands,
            wasm_sig.indirect_params,
            &mut bindgen,
        );
        for line in bindgen.into_body().lines() {
            writeln!(self.src, "      {line}").unwrap();
        }
        writeln!(self.src, "    }}").unwrap();
        writeln!(self.src, "  }}").unwrap();

        let return_type = func
            .result
            .as_ref()
            .map(|ty| self.type_ref(ty))
            .unwrap_or_else(|| "void".into());
        writeln!(self.src, "  finish(status: i32): {return_type} {{").unwrap();
        writeln!(self.src, "    if (this.finished) unreachable();").unwrap();
        writeln!(self.src, "    this.update(status);").unwrap();
        writeln!(
            self.src,
            "    if (this.state != async_.STATUS_RETURNED) unreachable();"
        )
        .unwrap();

        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestImportAsync,
            LiftLower::LowerArgsLiftResults,
            String::new(),
        );
        if let Some(result) = &func.result {
            let value = abi::lift_from_memory(
                bindgen.iface_gen.resolve,
                &mut bindgen,
                "this.result".into(),
                result,
            );
            bindgen.push_line(&format!("const value = {value};"));
        }
        bindgen.push_line("this.cleanup(false);");
        bindgen.push_line("this.release();");
        if func.result.is_some() {
            bindgen.push_line("return value;");
        }
        for line in bindgen.into_body().lines() {
            writeln!(self.src, "    {line}").unwrap();
        }
        writeln!(self.src, "  }}").unwrap();

        writeln!(self.src, "  dispose(status: i32): bool {{").unwrap();
        writeln!(self.src, "    if (this.finished) unreachable();").unwrap();
        writeln!(self.src, "    this.update(status);").unwrap();
        writeln!(
            self.src,
            "    const cancelled = this.state == async_.STATUS_STARTED_CANCELLED ||"
        )
        .unwrap();
        writeln!(
            self.src,
            "                      this.state == async_.STATUS_RETURNED_CANCELLED;"
        )
        .unwrap();
        writeln!(
            self.src,
            "    if (!cancelled && this.state != async_.STATUS_RETURNED) unreachable();"
        )
        .unwrap();
        writeln!(
            self.src,
            "    this.cleanup(this.state == async_.STATUS_RETURNED);"
        )
        .unwrap();
        writeln!(self.src, "    this.release();").unwrap();
        writeln!(self.src, "    return cancelled;").unwrap();
        writeln!(self.src, "  }}").unwrap();

        writeln!(self.src, "  private cleanup(disposeResult: bool): void {{").unwrap();
        writeln!(self.src, "    if (this.finished) unreachable();").unwrap();
        writeln!(self.src, "    this.finished = true;").unwrap();
        if let Some(result) = &func.result {
            writeln!(self.src, "    if (disposeResult) {{").unwrap();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestImportAsync,
                LiftLower::LowerArgsLiftResults,
                String::new(),
            );
            let mut params_only = func.clone();
            params_only.result = None;
            bindgen.next_endpoint = params_only
                .find_futures_and_streams(bindgen.iface_gen.resolve)
                .len();
            abi::deallocate_lists_and_own_in_types(
                bindgen.iface_gen.resolve,
                std::slice::from_ref(result),
                &["this.result".into()],
                true,
                &mut bindgen,
            );
            for line in bindgen.into_body().lines() {
                writeln!(self.src, "      {line}").unwrap();
            }
            // On the success path the result was lifted: lists and strings were
            // copied out, so their canonical buffers must still be freed, but
            // owned handles were transferred to the lifted value and must not be
            // dropped here.
            writeln!(self.src, "    }} else {{").unwrap();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestImportAsync,
                LiftLower::LowerArgsLiftResults,
                String::new(),
            );
            let mut params_only = func.clone();
            params_only.result = None;
            bindgen.next_endpoint = params_only
                .find_futures_and_streams(bindgen.iface_gen.resolve)
                .len();
            abi::deallocate_lists_in_types(
                bindgen.iface_gen.resolve,
                std::slice::from_ref(result),
                &["this.result".into()],
                true,
                &mut bindgen,
            );
            for line in bindgen.into_body().lines() {
                writeln!(self.src, "      {line}").unwrap();
            }
            writeln!(self.src, "    }}").unwrap();
            let layout = self.world_gen.sizes.record([result]);
            writeln!(
                self.src,
                "    ffi.cabi_realloc(this.result, {}, {}, 0);",
                layout.size.size_wasm32(),
                layout.align.align_wasm32()
            )
            .unwrap();
        }
        if wasm_sig.indirect_params {
            let layout = self
                .world_gen
                .sizes
                .record(func.params.iter().map(|p| &p.ty));
            writeln!(
                self.src,
                "    ffi.cabi_realloc(this.a0, {}, {}, 0);",
                layout.size.size_wasm32(),
                layout.align.align_wasm32()
            )
            .unwrap();
        }
        writeln!(self.src, "    if (this.handle != 0) {{").unwrap();
        writeln!(self.src, "      async_.waitableJoin(this.handle, 0);").unwrap();
        writeln!(self.src, "      async_.subtaskDrop(this.handle);").unwrap();
        writeln!(self.src, "      this.handle = 0;").unwrap();
        writeln!(self.src, "    }}").unwrap();
        writeln!(self.src, "  }}").unwrap();
        // The subtask must not free itself from inside its own method: the
        // caller still holds the pointer, and with the allocation released the
        // optimizer is free to reorder the remaining field accesses against the
        // free. Release it through an opaque call as the last statement of
        // `finish`/`dispose` instead.
        writeln!(self.src, "  @inline(false)").unwrap();
        writeln!(self.src, "  private release(): void {{").unwrap();
        writeln!(self.src, "    heap.free(changetype<usize>(this));").unwrap();
        writeln!(self.src, "  }}").unwrap();
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn gen_export_function(&mut self, func: &Function) {
        let func_local = Self::func_ident(&func.name);
        let user_ident = strip_resource_prefix(&func.name);
        let user_ident = ident::value_name(user_ident);

        // 1. Stub in exports/<iface>.ts: the user-edited entrypoint.
        let is_async = self.world_gen.opts.async_.is_async(
            self.resolve,
            self.interface_key.as_ref(),
            func,
            false,
        );
        if is_async {
            self.needs_async = true;
            self.world_gen.needs_async = true;
        }
        self.emit_future_stream_helpers(func, true);
        let variant = if is_async {
            AbiVariant::GuestExportAsync
        } else {
            AbiVariant::GuestExport
        };
        let sig = self.func_signature(func, variant);

        // An async export owns its parameters for the whole life of its task,
        // so their release is driven from the finish helper rather than from
        // the wrapper. Compute it up front: the task base has to carry the raw
        // arguments as fields for the finish helper to read back.
        let async_params = if is_async {
            self.async_export_param_cleanup(func, &user_ident)
        } else {
            None
        };

        if is_async {
            self.emit_async_task_base(func, &user_ident, &sig, async_params.as_ref());
        }

        // The user-facing entrypoint goes to `stubs/<basename>.ts`; the glue
        // below imports it by name. Signature types must be re-resolved in stub
        // scope, where this interface's own types live behind the glue
        // namespace.
        self.emitting_stub = true;
        let stub_sig = self.func_signature(func, variant);
        self.emitting_stub = false;
        let docs = func.docs.clone();
        let user = user_ident.clone();
        let glue_ns = self.glue_ns();
        self.with_stub(|g| {
            g.docs(&docs);
            if is_async {
                writeln!(
                    g.src,
                    "/// Return an `@unmanaged` state machine; persist only scalar handles/pointers across `resume` calls."
                )
                .unwrap();
            }
            let user_sig = if is_async {
                stub_sig.async_export_signature(&user, &format!("{glue_ns}."))
            } else {
                stub_sig.user_signature(&user)
            };
            writeln!(g.src, "export function {user_sig} {{").unwrap();
            if is_async {
                writeln!(
                    g.src,
                    "  return new {glue_ns}.{}Task();",
                    ident::type_name(&user)
                )
                .unwrap();
            } else {
                writeln!(g.src, "  // TODO: implement").unwrap();
                if let Some(ret_ty) = &stub_sig.return_type {
                    writeln!(g.src, "  return {};", default_value_for(ret_ty)).unwrap();
                }
            }
            writeln!(g.src, "}}").unwrap();
            writeln!(g.src).unwrap();
        });
        self.stub_values.insert(user_ident.clone());

        // 2. The wasm-export wrapper. Lives alongside the user stub in
        //    exports/<basename>.ts so it has direct access to the user function
        //    and any types defined in this file. bindings.ts plain-re-exports
        //    it by identifier; the post-compile rewriter renames the wasm
        //    export entry to the canonical WIT name.
        let wasm_name = self.wasm_export_name(func, is_async, WasmExportKind::Normal);
        let unique_as_name = format!(
            "__exp_{}",
            sanitize_extern_local(&format!(
                "{}_{}",
                self.interface.map(|id| id.index()).unwrap_or(usize::MAX),
                func_local
            ))
        );

        let wrapper_sig = sig.wasm_wrapper_signature(&unique_as_name);
        let mut wrapper_body = String::new();

        if is_async {
            let arg_count = async_params.as_ref().map(|p| p.0.len()).unwrap_or(0);
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                variant,
                LiftLower::LiftArgsLowerResults,
                user_ident.clone(),
            );
            bindgen.async_export_arg_count = arg_count;
            abi::call(
                bindgen.iface_gen.resolve,
                variant,
                LiftLower::LiftArgsLowerResults,
                func,
                &mut bindgen,
                true,
            );
            wrapper_body.push_str(&bindgen.into_body());
        } else {
            let cleanup = self.sync_export_param_cleanup(func);
            let user_call = user_ident.clone();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestExport,
                LiftLower::LiftArgsLowerResults,
                user_call,
            );
            bindgen.after_call = cleanup;
            abi::call(
                bindgen.iface_gen.resolve,
                AbiVariant::GuestExport,
                LiftLower::LiftArgsLowerResults,
                func,
                &mut bindgen,
                false,
            );
            wrapper_body.push_str(&bindgen.into_body());
        }

        // Keep lifting and the managed task.resume frame out of the raw wasm
        // export. task.return must run only after this helper has unwound.
        if is_async {
            let start_name = format!("__start_{unique_as_name}");
            writeln!(
                self.src,
                "@inline(false)\nfunction {} {{",
                sig.wasm_wrapper_signature(&start_name)
            )
            .unwrap();
            for line in wrapper_body.lines() {
                writeln!(self.src, "  {line}").unwrap();
            }
            writeln!(self.src, "}}").unwrap();
            writeln!(self.src).unwrap();
            wrapper_body = format!(
                "const __status = {start_name}({});\n__finish_{unique_as_name}(__status);\nreturn __status;",
                (0..sig.wasm_params.len())
                    .map(|i| format!("a{i}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        // Emit the wrapper into this file (exports/<basename>.ts).
        writeln!(self.src, "// wasm export: {wasm_name}").unwrap();
        writeln!(self.src, "export function {wrapper_sig} {{").unwrap();
        for line in wrapper_body.lines() {
            writeln!(self.src, "  {line}").unwrap();
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();

        // Record the rename mapping so bindings.ts re-exports + the wasm-tools
        // rewrite step can wire up the canonical name.
        let basename = self
            .interface
            .and_then(|id| self.world_gen.export_basenames.get(&id).cloned())
            .unwrap_or_default();
        self.world_gen.exports.push(ExportEntry {
            wasm_name,
            as_name: unique_as_name,
            basename,
            body: String::new(),
        });

        if is_async {
            self.emit_async_export_support(
                func,
                &func_local,
                &user_ident,
                async_params.as_ref().map(|p| p.1.as_str()).unwrap_or(""),
            );
        } else {
            self.emit_post_return(func, &func_local);
        }
    }

    /// Release what the caller transferred with an *async* export's parameters.
    ///
    /// Unlike a synchronous export, the task outlives the wrapper, so this runs
    /// from the finish helper on the way out — on both the returned and the
    /// cancelled path. Returns the raw wasm types the task base has to persist
    /// as `__arg<i>` fields, plus the cleanup body, or `None` when the
    /// parameters own nothing.
    fn async_export_param_cleanup(
        &mut self,
        func: &Function,
        user_ident: &str,
    ) -> Option<(Vec<WasmType>, String)> {
        let types: Vec<Type> = func.params.iter().map(|p| p.ty).collect();
        if types.is_empty() {
            return None;
        }
        let task_type = format!("{}Task", ident::type_name(user_ident));
        let wasm_sig = self
            .resolve
            .wasm_signature(AbiVariant::GuestExportAsync, func);
        let arg_types = wasm_sig.params.clone();
        let load = |i: usize| {
            format!(
                "load<{}>(task + offsetof<{task_type}>(\"__arg{i}\"))",
                wasm_type_name(arg_types[i])
            )
        };
        let operands: Vec<String> = if wasm_sig.indirect_params {
            vec![load(0)]
        } else {
            (0..arg_types.len()).map(load).collect()
        };

        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestExportAsync,
            LiftLower::LiftArgsLowerResults,
            String::new(),
        );
        bindgen.local_prefix = "p";
        bindgen.skip_endpoint_drops = true;
        abi::deallocate_lists_and_own_in_types(
            bindgen.iface_gen.resolve,
            &types,
            &operands,
            wasm_sig.indirect_params,
            &mut bindgen,
        );
        let body = bindgen.into_body();
        if body.trim().is_empty() {
            return None;
        }
        Some((arg_types, body))
    }

    /// Release what the caller transferred with a synchronous export's
    /// parameters: list and string buffers (lifting copies them, so nothing
    /// else frees them), error contexts, and owned handles.
    ///
    /// Emitted immediately after the user call rather than before the return,
    /// so that an owned exported-resource parameter handed straight back as the
    /// result is released *before* the result acquires its new handle.
    fn sync_export_param_cleanup(&mut self, func: &Function) -> String {
        let types: Vec<Type> = func.params.iter().map(|p| p.ty).collect();
        if types.is_empty() {
            return String::new();
        }
        let wasm_sig = self.resolve.wasm_signature(AbiVariant::GuestExport, func);
        let operands: Vec<String> = if wasm_sig.indirect_params {
            vec!["a0".to_string()]
        } else {
            (0..wasm_sig.params.len())
                .map(|i| format!("a{i}"))
                .collect()
        };
        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestExport,
            LiftLower::LiftArgsLowerResults,
            String::new(),
        );
        // Distinct local prefix: these lines are spliced into the wrapper body,
        // which numbers its own locals from `v0`.
        bindgen.local_prefix = "d";
        abi::deallocate_lists_and_own_in_types(
            bindgen.iface_gen.resolve,
            &types,
            &operands,
            wasm_sig.indirect_params,
            &mut bindgen,
        );
        bindgen.into_body()
    }

    /// `cabi_post_*`: frees the linear memory the export handed back. Without
    /// it every synchronous export returning a string or list leaks its
    /// returned buffers.
    fn emit_post_return(&mut self, func: &Function, func_local: &str) {
        if !abi::guest_export_needs_post_return(self.resolve, func) {
            return;
        }
        // A bare `-> error-context` result also reports "needs deallocate", but
        // it is a flat i32 transferred to the caller, and `abi::post_return`
        // requires a return pointer. Nothing to free in that case.
        if !self
            .resolve
            .wasm_signature(AbiVariant::GuestExport, func)
            .retptr
        {
            return;
        }
        let safe = sanitize_extern_local(&format!(
            "{}_{}",
            self.interface.map(|id| id.index()).unwrap_or(usize::MAX),
            func_local
        ));
        let as_name = format!("__post_return_{safe}");
        let wasm_name = self.wasm_export_name(func, false, WasmExportKind::PostReturn);

        let mut bindgen = FunctionBindgen::new(
            self,
            func,
            AbiVariant::GuestExport,
            LiftLower::LiftArgsLowerResults,
            String::new(),
        );
        abi::post_return(bindgen.iface_gen.resolve, func, &mut bindgen);
        let body = bindgen.into_body();

        writeln!(self.src, "// wasm export: {wasm_name}").unwrap();
        writeln!(self.src, "export function {as_name}(a0: usize): void {{").unwrap();
        for line in body.lines() {
            writeln!(self.src, "  {line}").unwrap();
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();

        let basename = self
            .interface
            .and_then(|id| self.world_gen.export_basenames.get(&id).cloned())
            .unwrap_or_default();
        self.world_gen.exports.push(ExportEntry {
            wasm_name,
            as_name,
            basename,
            body: String::new(),
        });
    }

    fn emit_async_task_base(
        &mut self,
        func: &Function,
        func_local: &str,
        sig: &FuncSig,
        async_params: Option<&(Vec<WasmType>, String)>,
    ) {
        let task_type = format!("{}Task", ident::type_name(func_local));
        let (_, _, task_return_sig) =
            func.task_return_import(self.resolve, self.interface_key.as_ref(), Mangling::Legacy);
        let task_return_types = task_return_sig.params;
        writeln!(
            self.src,
            "@unmanaged\nexport class {task_type} extends async_.AsyncTask {{"
        )
        .unwrap();
        writeln!(
            self.src,
            "  resume(_event: i32, _waitable: i32, _code: i32): i32 {{"
        )
        .unwrap();
        writeln!(self.src, "    unreachable();").unwrap();
        writeln!(self.src, "    return async_.CALLBACK_CODE_EXIT;").unwrap();
        writeln!(self.src, "  }}").unwrap();
        writeln!(self.src, "  finished: bool = false;").unwrap();
        if let Some((arg_types, _)) = async_params {
            writeln!(
                self.src,
                "  // Raw arguments, kept so the finish helper can release what"
            )
            .unwrap();
            writeln!(self.src, "  // the caller transferred with them.").unwrap();
            for (i, ty) in arg_types.iter().enumerate() {
                writeln!(self.src, "  __arg{i}: {} = 0;", wasm_type_name(*ty)).unwrap();
            }
        }
        self.emit_typed_future_helpers(func);
        for (i, ty) in task_return_types.iter().enumerate() {
            writeln!(self.src, "  return{i}: {} = 0;", wasm_type_name(*ty)).unwrap();
        }
        if func.result.is_some() {
            let result_ty = sig.return_type.as_deref().unwrap();
            writeln!(self.src, "  finish(result: {result_ty}): i32 {{").unwrap();
            let result = func.result.as_ref().unwrap();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestExportAsync,
                LiftLower::LowerArgsLiftResults,
                "this.storeReturn".into(),
            );
            let args = if task_return_types == [WasmType::Pointer] {
                let layout = bindgen.iface_gen.world_gen.sizes.record([result]);
                bindgen.push_line(&format!(
                    "const __result = ffi.cabi_realloc(0, 0, {}, {});",
                    layout.align.align_wasm32(),
                    layout.size.size_wasm32()
                ));
                abi::lower_to_memory(
                    bindgen.iface_gen.resolve,
                    &mut bindgen,
                    "__result".into(),
                    "result".into(),
                    result,
                );
                vec!["__result".into()]
            } else {
                abi::lower_flat(
                    bindgen.iface_gen.resolve,
                    &mut bindgen,
                    "result".into(),
                    result,
                )
            };
            bindgen.push_line(&format!("this.storeReturn({});", args.join(", ")));
            for line in bindgen.into_body().lines() {
                writeln!(self.src, "    {line}").unwrap();
            }
        } else {
            writeln!(self.src, "  finish(): i32 {{").unwrap();
        }
        writeln!(self.src, "    this.finished = true;").unwrap();
        writeln!(self.src, "    return async_.CALLBACK_CODE_EXIT;").unwrap();
        writeln!(self.src, "  }}").unwrap();
        if !task_return_types.is_empty() {
            let params = task_return_types
                .iter()
                .enumerate()
                .map(|(i, ty)| format!("a{i}: {}", wasm_type_name(*ty)))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(self.src, "  private storeReturn({params}): void {{").unwrap();
            for i in 0..task_return_types.len() {
                writeln!(self.src, "    this.return{i} = a{i};").unwrap();
            }
            writeln!(self.src, "  }}").unwrap();
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn emit_typed_future_helpers(&mut self, func: &Function) {
        for (index, ty) in func
            .find_futures_and_streams(self.resolve)
            .into_iter()
            .enumerate()
        {
            let TypeDefKind::Future(payload) = &self.resolve.types[ty].kind else {
                continue;
            };
            let payload = *payload;
            let stem = format!(
                "rawExport{}Future{index}",
                ident::type_name(&Self::func_ident(&func.name))
            );
            let field = format!("future{index}Payload");

            if let Some(payload) = payload {
                let layout = self.world_gen.sizes.record([&payload]);
                writeln!(self.src, "  private {field}: usize = 0;").unwrap();
                writeln!(self.src, "  startFuture{index}Read(handle: i32): i32 {{").unwrap();
                writeln!(
                    self.src,
                    "    if (this.{field} == 0) this.{field} = ffi.cabi_realloc(0, 0, {}, {});",
                    layout.align.align_wasm32(),
                    layout.size.size_wasm32()
                )
                .unwrap();
                writeln!(self.src, "    return {stem}Read(handle, this.{field});").unwrap();
                writeln!(self.src, "  }}").unwrap();

                let result_ty = self.type_ref(&payload);
                writeln!(self.src, "  finishFuture{index}Read(): {result_ty} {{").unwrap();
                let mut bindgen = FunctionBindgen::new(
                    self,
                    func,
                    AbiVariant::GuestExportAsync,
                    LiftLower::LowerArgsLiftResults,
                    String::new(),
                );
                let value = abi::lift_from_memory(
                    bindgen.iface_gen.resolve,
                    &mut bindgen,
                    format!("this.{field}"),
                    &payload,
                );
                bindgen.push_line(&format!("const value = {value};"));
                abi::deallocate_lists_in_types(
                    bindgen.iface_gen.resolve,
                    &[payload],
                    &[format!("this.{field}")],
                    true,
                    &mut bindgen,
                );
                for line in bindgen.into_body().lines() {
                    writeln!(self.src, "    {line}").unwrap();
                }
                writeln!(self.src, "    const ptr = this.{field};").unwrap();
                writeln!(self.src, "    this.{field} = 0;").unwrap();
                writeln!(
                    self.src,
                    "    ffi.cabi_realloc(ptr, {}, {}, 0);",
                    layout.size.size_wasm32(),
                    layout.align.align_wasm32()
                )
                .unwrap();
                writeln!(self.src, "    return value;").unwrap();
                writeln!(self.src, "  }}").unwrap();

                writeln!(
                    self.src,
                    "  startFuture{index}Write(handle: i32, value: {result_ty}): i32 {{"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "    if (this.{field} == 0) this.{field} = ffi.cabi_realloc(0, 0, {}, {});",
                    layout.align.align_wasm32(),
                    layout.size.size_wasm32()
                )
                .unwrap();
                let mut bindgen = FunctionBindgen::new(
                    self,
                    func,
                    AbiVariant::GuestExportAsync,
                    LiftLower::LowerArgsLiftResults,
                    String::new(),
                );
                abi::lower_to_memory(
                    bindgen.iface_gen.resolve,
                    &mut bindgen,
                    format!("this.{field}"),
                    "value".into(),
                    &payload,
                );
                for line in bindgen.into_body().lines() {
                    writeln!(self.src, "    {line}").unwrap();
                }
                writeln!(self.src, "    return {stem}Write(handle, this.{field});").unwrap();
                writeln!(self.src, "  }}").unwrap();
                writeln!(self.src, "  finishFuture{index}Write(): void {{").unwrap();
                let mut bindgen = FunctionBindgen::new(
                    self,
                    func,
                    AbiVariant::GuestExportAsync,
                    LiftLower::LowerArgsLiftResults,
                    String::new(),
                );
                abi::deallocate_lists_in_types(
                    bindgen.iface_gen.resolve,
                    &[payload],
                    &[format!("this.{field}")],
                    true,
                    &mut bindgen,
                );
                for line in bindgen.into_body().lines() {
                    writeln!(self.src, "    {line}").unwrap();
                }
                writeln!(self.src, "    const ptr = this.{field};").unwrap();
                writeln!(self.src, "    this.{field} = 0;").unwrap();
                writeln!(
                    self.src,
                    "    if (ptr != 0) ffi.cabi_realloc(ptr, {}, {}, 0);",
                    layout.size.size_wasm32(),
                    layout.align.align_wasm32()
                )
                .unwrap();
                writeln!(self.src, "  }}").unwrap();
            } else {
                writeln!(self.src, "  startFuture{index}Read(handle: i32): i32 {{").unwrap();
                writeln!(self.src, "    return {stem}Read(handle, 0);").unwrap();
                writeln!(self.src, "  }}").unwrap();
                writeln!(self.src, "  finishFuture{index}Read(): void {{}}").unwrap();
                writeln!(self.src, "  startFuture{index}Write(handle: i32): i32 {{").unwrap();
                writeln!(self.src, "    return {stem}Write(handle, 0);").unwrap();
                writeln!(self.src, "  }}").unwrap();
                writeln!(self.src, "  finishFuture{index}Write(): void {{}}").unwrap();
            }
        }
    }

    fn emit_async_export_support(
        &mut self,
        func: &Function,
        func_local: &str,
        user_ident: &str,
        param_cleanup: &str,
    ) {
        let safe = sanitize_extern_local(&format!(
            "{}_{}",
            self.interface.map(|id| id.index()).unwrap_or(usize::MAX),
            func_local
        ));
        let task_return_extern = format!("__task_return_{safe}");
        let task_type = format!("{}Task", ident::type_name(user_ident));
        let finish_name = format!("__finish___exp_{safe}");
        let (module, task_return_name, task_return_sig) =
            func.task_return_import(self.resolve, self.interface_key.as_ref(), Mangling::Legacy);
        let task_return_types = task_return_sig.params;
        let raw_params = task_return_types
            .iter()
            .enumerate()
            .map(|(i, ty)| format!("a{i}: {}", wasm_type_name(*ty)))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(self.src, "@external(\"{module}\", \"{task_return_name}\")").unwrap();
        writeln!(
            self.src,
            "declare function {task_return_extern}({raw_params}): void;"
        )
        .unwrap();

        let return_args = (0..task_return_types.len())
            .map(|i| {
                format!(
                    "load<{}>(task + offsetof<{task_type}>(\"return{i}\"))",
                    wasm_type_name(task_return_types[i])
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            self.src,
            "@inline(false)\nexport function {finish_name}(status: i32): void {{"
        )
        .unwrap();
        writeln!(self.src, "  const task = async_.contextGet();").unwrap();
        writeln!(
            self.src,
            "  if (status != async_.CALLBACK_CODE_EXIT || task == 0) return;"
        )
        .unwrap();
        // The canonical ABI requires an exiting task to have performed exactly
        // one of `task.return` or `task.cancel`. Issue whichever the task did
        // not, and trap on an exit that is neither, rather than letting the host
        // trap with no guest context.
        writeln!(
            self.src,
            "  if (load<bool>(task + offsetof<{task_type}>(\"finished\"))) {{"
        )
        .unwrap();
        writeln!(self.src, "    {task_return_extern}({return_args});").unwrap();
        writeln!(
            self.src,
            "  }} else if (async_.Scheduler.wasCancelled()) {{"
        )
        .unwrap();
        writeln!(self.src, "    async_.taskCancel();").unwrap();
        writeln!(self.src, "  }} else {{").unwrap();
        writeln!(self.src, "    unreachable();").unwrap();
        writeln!(self.src, "  }}").unwrap();
        // The task owned its parameters for its whole life; release them on the
        // way out, whichever way it exited, while the task is still allocated.
        for line in param_cleanup.lines() {
            writeln!(self.src, "  {line}").unwrap();
        }
        // Clear context-0 before freeing the task so `complete` never inspects
        // a freed allocation.
        writeln!(self.src, "  async_.Scheduler.complete(task);").unwrap();
        writeln!(self.src, "  async_.Scheduler.release(task);").unwrap();
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();

        let callback_name = self.wasm_export_name(func, true, WasmExportKind::Callback);
        let callback_as_name = format!("__callback_{safe}");
        writeln!(
            self.src,
            "export function {callback_as_name}(event: i32, waitable: i32, code: i32): i32 {{"
        )
        .unwrap();
        writeln!(
            self.src,
            "  const status = async_.Scheduler.resume(event, waitable, code);"
        )
        .unwrap();
        writeln!(self.src, "  {finish_name}(status);").unwrap();
        writeln!(self.src, "  return status;").unwrap();
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
        let basename = self
            .interface
            .and_then(|id| self.world_gen.export_basenames.get(&id).cloned())
            .unwrap_or_default();
        self.world_gen.exports.push(ExportEntry {
            wasm_name: callback_name,
            as_name: callback_as_name,
            basename,
            body: String::new(),
        });
    }

    fn emit_future_stream_helpers(&mut self, func: &Function, exported: bool) {
        for (index, ty) in func
            .find_futures_and_streams(self.resolve)
            .into_iter()
            .enumerate()
        {
            let (kind, intrinsic_ty) = match &self.resolve.types[ty].kind {
                TypeDefKind::Future(payload) => {
                    (EndpointKind::Future, payload.as_ref().map(|_| ty))
                }
                TypeDefKind::Stream(payload) => {
                    (EndpointKind::Stream, payload.as_ref().map(|_| ty))
                }
                _ => unreachable!(),
            };
            self.emit_future_stream_helper(func, exported, index, kind, intrinsic_ty);
        }
    }

    fn emit_future_stream_helper(
        &mut self,
        func: &Function,
        exported: bool,
        index: usize,
        kind: EndpointKind,
        intrinsic_ty: Option<TypeId>,
    ) {
        let stem = format!(
            "raw{}{}{}{index}",
            if exported { "Export" } else { "Import" },
            ident::type_name(&Self::func_ident(&func.name)),
            kind.type_name()
        );
        for intrinsic in EndpointIntrinsic::ALL {
            let import = match kind {
                EndpointKind::Future => WasmImport::FutureIntrinsic {
                    interface: self.interface_key.as_ref(),
                    func,
                    ty: intrinsic_ty,
                    intrinsic: intrinsic.future(),
                    exported,
                    async_: intrinsic.async_lowered(),
                },
                EndpointKind::Stream => WasmImport::StreamIntrinsic {
                    interface: self.interface_key.as_ref(),
                    func,
                    ty: intrinsic_ty,
                    intrinsic: intrinsic.stream(),
                    exported,
                    async_: intrinsic.async_lowered(),
                },
            };
            let (module, field) = self
                .resolve
                .wasm_import_name(ManglingAndAbi::Legacy(LiftLowerAbi::Sync), import);
            let operation = intrinsic.type_name();
            let extern_name = format!("__{stem}{operation}");
            let helper_name = format!("{stem}{operation}");
            let (params, args, result) = intrinsic.signature(kind);
            writeln!(self.src, "@external(\"{module}\", \"{field}\")").unwrap();
            writeln!(
                self.src,
                "declare function {extern_name}({params}): {result};"
            )
            .unwrap();
            writeln!(
                self.src,
                "export function {helper_name}({params}): {result} {{ {}{extern_name}({args}); }}",
                if result == "void" { "" } else { "return " }
            )
            .unwrap();
        }
        writeln!(self.src).unwrap();
    }

    fn wasm_export_name(&self, func: &Function, is_async: bool, kind: WasmExportKind) -> String {
        self.resolve.wasm_export_name(
            ManglingAndAbi::Legacy(if is_async {
                LiftLowerAbi::AsyncCallback
            } else {
                LiftLowerAbi::Sync
            }),
            WasmExport::Func {
                interface: self.interface_key.as_ref(),
                func,
                kind,
            },
        )
    }

    fn func_signature(&mut self, func: &Function, variant: AbiVariant) -> FuncSig {
        let params: Vec<(String, String)> = func
            .params
            .iter()
            .map(|p| (Self::field_ident(&p.name), self.type_ref(&p.ty)))
            .collect();
        let return_type = func.result.as_ref().map(|t| self.type_ref(t));
        let wasm_sig = self.resolve.wasm_signature(variant, func);
        FuncSig {
            params,
            return_type,
            wasm_params: wasm_sig.params.clone(),
            wasm_results: wasm_sig.results.clone(),
        }
    }
}

#[derive(Clone, Copy)]
enum EndpointKind {
    Future,
    Stream,
}

impl EndpointKind {
    fn type_name(self) -> &'static str {
        match self {
            Self::Future => "Future",
            Self::Stream => "Stream",
        }
    }
}

#[derive(Clone, Copy)]
enum EndpointIntrinsic {
    New,
    Read,
    Write,
    CancelRead,
    CancelWrite,
    DropReadable,
    DropWritable,
}

impl EndpointIntrinsic {
    const ALL: [Self; 7] = [
        Self::New,
        Self::Read,
        Self::Write,
        Self::CancelRead,
        Self::CancelWrite,
        Self::DropReadable,
        Self::DropWritable,
    ];

    fn type_name(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Read => "Read",
            Self::Write => "Write",
            Self::CancelRead => "CancelRead",
            Self::CancelWrite => "CancelWrite",
            Self::DropReadable => "DropReadable",
            Self::DropWritable => "DropWritable",
        }
    }

    fn async_lowered(self) -> bool {
        matches!(self, Self::Read | Self::Write)
    }

    fn future(self) -> FutureIntrinsic {
        match self {
            Self::New => FutureIntrinsic::New,
            Self::Read => FutureIntrinsic::Read,
            Self::Write => FutureIntrinsic::Write,
            Self::CancelRead => FutureIntrinsic::CancelRead,
            Self::CancelWrite => FutureIntrinsic::CancelWrite,
            Self::DropReadable => FutureIntrinsic::DropReadable,
            Self::DropWritable => FutureIntrinsic::DropWritable,
        }
    }

    fn stream(self) -> StreamIntrinsic {
        match self {
            Self::New => StreamIntrinsic::New,
            Self::Read => StreamIntrinsic::Read,
            Self::Write => StreamIntrinsic::Write,
            Self::CancelRead => StreamIntrinsic::CancelRead,
            Self::CancelWrite => StreamIntrinsic::CancelWrite,
            Self::DropReadable => StreamIntrinsic::DropReadable,
            Self::DropWritable => StreamIntrinsic::DropWritable,
        }
    }

    fn signature(self, kind: EndpointKind) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::New => ("", "", "i64"),
            Self::Read | Self::Write => match kind {
                EndpointKind::Future => ("handle: i32, payload: usize", "handle, payload", "i32"),
                EndpointKind::Stream => (
                    "handle: i32, payload: usize, count: usize",
                    "handle, payload, count",
                    "i32",
                ),
            },
            Self::CancelRead | Self::CancelWrite => ("handle: i32", "handle", "i32"),
            Self::DropReadable | Self::DropWritable => ("handle: i32", "handle", "void"),
        }
    }
}

struct FuncSig {
    params: Vec<(String, String)>,
    return_type: Option<String>,
    wasm_params: Vec<WasmType>,
    wasm_results: Vec<WasmType>,
}

impl FuncSig {
    fn user_signature(&self, name: &str) -> String {
        let ps: Vec<String> = self
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect();
        let ret = self.return_type.as_deref().unwrap_or("void");
        format!("{name}({}): {ret}", ps.join(", "))
    }

    fn wrapper_signature(&self, name: &str) -> String {
        self.user_signature(name)
    }

    fn async_import_signature(&self, name: &str, func_local: &str) -> String {
        let ps = self
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{name}({ps}): {}Subtask", ident::type_name(func_local))
    }

    /// `qualifier` prefixes the task type, which is defined in the generated
    /// glue file rather than alongside the user's implementation.
    fn async_export_signature(&self, name: &str, qualifier: &str) -> String {
        let ps = self
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{name}({ps}): {qualifier}{}Task", ident::type_name(name))
    }

    fn wasm_wrapper_signature(&self, name: &str) -> String {
        let ps: Vec<String> = self
            .wasm_params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("a{i}: {}", wasm_type_name(*t)))
            .collect();
        let ret = self
            .wasm_return_type()
            .unwrap_or_else(|| "void".to_string());
        format!("{name}({}): {ret}", ps.join(", "))
    }

    fn wasm_return_type(&self) -> Option<String> {
        if self.wasm_results.is_empty() {
            None
        } else if self.wasm_results.len() == 1 {
            Some(wasm_type_name(self.wasm_results[0]).to_string())
        } else {
            // multi-value return: handled via out-pointer; no AS return value
            None
        }
    }
}

/// If `res_id` belongs to a different interface than the current
/// InterfaceGenerator's, return the `e_<basename>.` namespace prefix to reach
/// the handle table that interface's `exports/<basename>.ts` defined.
fn resource_table_prefix(g: &mut InterfaceGenerator<'_>, res_id: TypeId) -> String {
    let ty = &g.resolve.types[res_id];
    match ty.owner {
        TypeOwner::Interface(other) if Some(other) != g.interface => {
            if let Some(bn) = g.world_gen.export_basenames.get(&other).cloned() {
                g.imports_exports.insert(bn.clone());
                return format!("e_{bn}.");
            }
        }
        _ => {}
    }
    String::new()
}

/// Default-value expression for the given AS type name. Used as a placeholder
/// for "uninitialised" slots in Option<T> and similar.
fn default_value_for(as_type: &str) -> String {
    match as_type.trim() {
        "bool" => "false".into(),
        "u8" | "i8" | "u16" | "i16" | "u32" | "i32" | "char" => "0".into(),
        "u64" | "i64" => "<i64>0".into(),
        "f32" => "0.0".into(),
        "f64" => "0.0".into(),
        "string" => "\"\"".into(),
        "usize" => "0".into(),
        other => format!("changetype<{other}>(0)"),
    }
}

fn tuple_ref<'a>(g: &mut InterfaceGenerator<'a>, tuple: &Tuple) -> String {
    let arity = tuple.types.len();
    if arity == 0 {
        return "void".into();
    }
    if arity > 16 {
        panic!(
            "tuple<...> arity {arity} > 16 not supported by AS backend; \
             consider modeling as a record."
        );
    }
    let parts: Vec<String> = tuple.types.iter().map(|t| g.type_ref(t)).collect();
    format!("ffi.Tuple{arity}<{}>", parts.join(", "))
}

fn wasm_type_name(ty: WasmType) -> &'static str {
    match ty {
        WasmType::I32 => "i32",
        WasmType::I64 => "i64",
        WasmType::F32 => "f32",
        WasmType::F64 => "f64",
        WasmType::Pointer => "usize",
        WasmType::Length => "usize",
        WasmType::PointerOrI64 => "i64",
    }
}

fn sanitize_extern_local(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn strip_resource_prefix(name: &str) -> &str {
    // names like `[method]foo.bar`, `[constructor]foo`, `[static]foo.bar`
    if let Some(rest) = name.strip_prefix("[method]") {
        return rest.split_once('.').map(|(_, m)| m).unwrap_or(rest);
    }
    if let Some(rest) = name.strip_prefix("[static]") {
        return rest.split_once('.').map(|(_, m)| m).unwrap_or(rest);
    }
    if let Some(rest) = name.strip_prefix("[constructor]") {
        return rest;
    }
    if let Some(rest) = name.strip_prefix("[resource-drop]") {
        return rest;
    }
    name
}

// =============================================================================
// CoreInterfaceGenerator (type emission)
// =============================================================================

impl<'a> CoreInterfaceGenerator<'a> for InterfaceGenerator<'a> {
    fn resolve(&self) -> &'a Resolve {
        self.resolve
    }

    fn type_record(&mut self, _id: TypeId, name: &str, record: &Record, docs: &Docs) {
        self.docs(docs);
        let name = ident::type_name(name);
        let fields: Vec<(String, String, Docs)> = record
            .fields
            .iter()
            .map(|f| {
                (
                    Self::field_ident(&f.name),
                    self.type_ref(&f.ty),
                    f.docs.clone(),
                )
            })
            .collect();
        writeln!(self.src, "export class {name} {{").unwrap();
        let params: Vec<String> = fields
            .iter()
            .map(|(n, t, _)| format!("public {n}: {t}"))
            .collect();
        writeln!(self.src, "  constructor({}) {{}}", params.join(", ")).unwrap();
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_resource(&mut self, id: TypeId, name: &str, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        match self.direction {
            Direction::Import => {
                // Wrap the handle. @unmanaged removes GC tracking.
                let drop_module = match &self.interface_key {
                    Some(key) => self.resolve.name_world_key(key),
                    None => "$root".to_string(),
                };
                writeln!(
                    self.src,
                    "@external(\"{drop_module}\", \"[resource-drop]{name}\")"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "declare function __ext_{}_drop(h: i32): void;",
                    sanitize_extern_local(&cls)
                )
                .unwrap();
                writeln!(self.src).unwrap();
                writeln!(self.src, "@unmanaged").unwrap();
                writeln!(self.src, "export class {cls} {{").unwrap();
                writeln!(self.src, "  constructor(public handle: i32) {{}}").unwrap();
                writeln!(
                    self.src,
                    "  drop(): void {{ __ext_{}_drop(this.handle); }}",
                    sanitize_extern_local(&cls)
                )
                .unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src).unwrap();
            }
            Direction::Export => {
                self.world_gen.exported_resources.push(id);
                let resource_name = name.to_string();
                let resource_cls = cls.clone();
                self.with_stub(|g| {
                    writeln!(
                        g.src,
                        "// Exported resource `{resource_name}` — implement this class and"
                    )
                    .unwrap();
                    writeln!(g.src, "// add an optional `__onDrop()` method for cleanup.").unwrap();
                    writeln!(g.src, "export class {resource_cls} {{").unwrap();
                    writeln!(g.src, "  constructor() {{ /* user fields */ }}").unwrap();
                    writeln!(g.src, "  __onDrop(): void {{}}").unwrap();
                    writeln!(g.src, "}}").unwrap();
                    writeln!(g.src).unwrap();
                });
                self.stub_types.insert(cls.clone());

                // The handle table + new/drop wrappers, inlined in the same file
                // so they have direct access to the class:
                let safe = sanitize_extern_local(&cls);
                writeln!(
                    self.src,
                    "const __{safe}_table: Map<i32, {cls}> = new Map<i32, {cls}>();"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "const __{safe}_handles: Map<{cls}, i32> = new Map<{cls}, i32>();"
                )
                .unwrap();
                writeln!(self.src, "let __{safe}_next: i32 = 1;").unwrap();
                writeln!(
                    self.src,
                    "export function __{safe}_take(inst: {cls}): i32 {{"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "  const h = __{safe}_next++; __{safe}_table.set(h, inst); __{safe}_handles.set(inst, h); return h;"
                )
                .unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src, "export function __{safe}_get(h: i32): {cls} {{ return __{safe}_table.get(h)!; }}").unwrap();
                writeln!(self.src, "export function __{safe}_drop(h: i32): void {{").unwrap();
                writeln!(self.src, "  const inst = __{safe}_table.get(h);").unwrap();
                writeln!(
                    self.src,
                    "  if (inst !== null) {{ inst.__onDrop(); __{safe}_table.delete(h); __{safe}_handles.delete(inst); }}"
                )
                .unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(
                    self.src,
                    "export function __{safe}_drop_instance(inst: {cls}): void {{"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "  const h = __{safe}_handles.get(inst); if (h !== null) __{safe}_drop(h);"
                )
                .unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src).unwrap();

                // Register the canonical wasm-export `[dtor]<resource>` wrapper.
                let key = self
                    .interface_key
                    .as_ref()
                    .expect("world-level exported resource without an enclosing interface");
                let wasm_name = self.resolve.wasm_export_name(
                    ManglingAndAbi::Legacy(wit_bindgen_core::wit_parser::LiftLowerAbi::Sync),
                    wit_bindgen_core::wit_parser::WasmExport::ResourceDtor {
                        interface: key,
                        resource: id,
                    },
                );
                let as_name = format!("__exp_dtor_{safe}");
                writeln!(
                    self.src,
                    "// wasm export: {wasm_name} (resource destructor)"
                )
                .unwrap();
                writeln!(self.src, "export function {as_name}(h: i32): void {{").unwrap();
                writeln!(self.src, "  __{safe}_drop(h);").unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src).unwrap();
                let basename = self
                    .interface
                    .and_then(|iid| self.world_gen.export_basenames.get(&iid).cloned())
                    .unwrap_or_default();
                self.world_gen.exports.push(ExportEntry {
                    wasm_name,
                    as_name,
                    basename,
                    body: String::new(),
                });
            }
        }
    }

    fn type_flags(&mut self, _id: TypeId, name: &str, flags: &Flags, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let underlying = match flags.repr() {
            FlagsRepr::U8 => "u8",
            FlagsRepr::U16 => "u16",
            FlagsRepr::U32(1) => "u32",
            FlagsRepr::U32(2) => "u64",
            _ => panic!("WIT flag count > 64 unsupported by AssemblyScript backend"),
        };
        writeln!(self.src, "export class {cls} {{").unwrap();
        writeln!(
            self.src,
            "  constructor(public bits: {underlying} = 0) {{}}"
        )
        .unwrap();
        for (i, flag) in flags.flags.iter().enumerate() {
            let fname = ident::case_name(&flag.name);
            let upper = fname.to_uppercase();
            writeln!(
                self.src,
                "  static {upper}: {underlying} = <{underlying}>(1{} << {i});",
                if underlying == "u64" { "" } else { "" }
            )
            .unwrap();
        }
        writeln!(
            self.src,
            "  has(mask: {underlying}): bool {{ return (this.bits & mask) == mask; }}"
        )
        .unwrap();
        writeln!(
            self.src,
            "  set(mask: {underlying}): {cls} {{ return new {cls}(this.bits | mask); }}"
        )
        .unwrap();
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_tuple(&mut self, _id: TypeId, name: &str, tuple: &Tuple, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let alias = tuple_ref(self, tuple);
        writeln!(self.src, "export type {cls} = {alias};").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_variant(&mut self, _id: TypeId, name: &str, variant: &Variant, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        writeln!(self.src, "export class {cls} {{").unwrap();
        writeln!(self.src, "  constructor(public tag: i32) {{}}").unwrap();
        writeln!(self.src, "}}").unwrap();
        for (i, case) in variant.cases.iter().enumerate() {
            let cname = ident::case_name(&case.name);
            let payload_ty = case
                .ty
                .as_ref()
                .map(|t| self.type_ref(t))
                .unwrap_or_else(|| "void".to_string());
            writeln!(self.src, "export class {cls}_{cname} extends {cls} {{").unwrap();
            if case.ty.is_some() {
                writeln!(
                    self.src,
                    "  constructor(public value: {payload_ty}) {{ super({i}); }}"
                )
                .unwrap();
            } else {
                writeln!(self.src, "  constructor() {{ super({i}); }}").unwrap();
            }
            writeln!(self.src, "}}").unwrap();
        }
        writeln!(self.src).unwrap();
    }

    fn type_option(&mut self, _id: TypeId, name: &str, payload: &Type, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let pt = self.type_ref(payload);
        writeln!(self.src, "export type {cls} = ffi.Option<{pt}>;").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_result(&mut self, _id: TypeId, name: &str, r: &Result_, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let ok_ty =
            r.ok.as_ref()
                .map(|t| self.type_ref(t))
                .unwrap_or_else(|| "i32".to_string());
        let err_ty = r
            .err
            .as_ref()
            .map(|t| self.type_ref(t))
            .unwrap_or_else(|| "i32".to_string());
        writeln!(
            self.src,
            "export type {cls} = ffi.Result<{ok_ty}, {err_ty}>;"
        )
        .unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_enum(&mut self, _id: TypeId, name: &str, enum_: &Enum, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        writeln!(self.src, "export enum {cls} {{").unwrap();
        for (i, case) in enum_.cases.iter().enumerate() {
            let cname = ident::case_name(&case.name);
            writeln!(self.src, "  {cname} = {i},").unwrap();
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_alias(&mut self, _id: TypeId, name: &str, ty: &Type, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let t = self.type_ref(ty);
        writeln!(self.src, "export type {cls} = {t};").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_list(&mut self, _id: TypeId, name: &str, ty: &Type, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let t = self.type_ref(ty);
        // Use the per-element-type AS-native typed array where applicable.
        let aliased = match ty {
            Type::U8 => "Uint8Array".into(),
            Type::S8 => "Int8Array".into(),
            Type::U16 => "Uint16Array".into(),
            Type::S16 => "Int16Array".into(),
            Type::U32 => "Uint32Array".into(),
            Type::S32 => "Int32Array".into(),
            Type::U64 => "Uint64Array".into(),
            Type::S64 => "Int64Array".into(),
            Type::F32 => "Float32Array".into(),
            Type::F64 => "Float64Array".into(),
            _ => format!("Array<{t}>"),
        };
        writeln!(self.src, "export type {cls} = {aliased};").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_fixed_length_list(
        &mut self,
        _id: TypeId,
        name: &str,
        ty: &Type,
        _size: u32,
        docs: &Docs,
    ) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let t = self.type_ref(ty);
        writeln!(self.src, "export type {cls} = StaticArray<{t}>;").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_map(&mut self, _id: TypeId, name: &str, key: &Type, value: &Type, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let k = self.type_ref(key);
        let v = self.type_ref(value);
        writeln!(self.src, "export type {cls} = Map<{k}, {v}>;").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_builtin(&mut self, _id: TypeId, name: &str, ty: &Type, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        let t = self.type_ref(ty);
        writeln!(self.src, "export type {cls} = {t};").unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_future(&mut self, _id: TypeId, name: &str, _ty: &Option<Type>, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        writeln!(
            self.src,
            "export type {cls} = i32; /* future<T> handle; async not implemented */"
        )
        .unwrap();
        writeln!(self.src).unwrap();
    }

    fn type_stream(&mut self, _id: TypeId, name: &str, _ty: &Option<Type>, docs: &Docs) {
        self.docs(docs);
        let cls = ident::type_name(name);
        writeln!(
            self.src,
            "export type {cls} = i32; /* stream<T> handle; async not implemented */"
        )
        .unwrap();
        writeln!(self.src).unwrap();
    }
}

// =============================================================================
// FunctionBindgen (ABI lowering)
// =============================================================================

struct FunctionBindgen<'a, 'b> {
    iface_gen: &'a mut InterfaceGenerator<'b>,
    func: &'a Function,
    #[allow(dead_code, reason = "kept for future ABI work")]
    variant: AbiVariant,
    #[allow(dead_code, reason = "kept for future ABI work")]
    lift_lower: LiftLower,
    /// AS expression invoked when `CallWasm` is emitted (lower → wasm import) or
    /// the user-facing function name invoked when `CallInterface` is emitted
    /// (lift → user code).
    call_target: String,
    /// Counter for fresh local-variable names.
    next_local: u32,
    /// Prefix for fresh local names. Bodies generated separately and spliced
    /// into another body need their own prefix to avoid redeclaration.
    local_prefix: &'static str,
    /// Lines emitted straight after `CallInterface` on the synchronous export
    /// path — parameter cleanup, which must run after the user call but before
    /// the result is lowered.
    after_call: String,
    /// Lines emitted straight after `CallWasm` — release of buffers a
    /// synchronous import only borrowed.
    after_wasm_call: String,
    /// Number of raw wasm arguments an async export persists on its task so
    /// the finish helper can release them.
    async_export_arg_count: usize,
    /// Suppress `DropHandle` for future/stream endpoints.
    ///
    /// An async export's task drives its endpoints across suspensions and drops
    /// them itself, exactly as `test.c` does; releasing them again when the
    /// task exits would double-drop. Owned resources, lists, strings, and error
    /// contexts are still released.
    skip_endpoint_drops: bool,
    /// Next future/stream occurrence in the function's canonical endpoint order.
    next_endpoint: usize,
    /// Output source written into the current block. Blocks accumulate into
    /// `block_stack` when `push_block`/`finish_block` is called.
    src: String,
    /// Stack of completed blocks awaiting consumption by the lift/lower
    /// instruction that pushed them.
    blocks: Vec<(String, Vec<String>)>,
    /// Number of `push_block` calls active.
    block_storage: Vec<(String, Vec<String>)>,
}

impl<'a, 'b> FunctionBindgen<'a, 'b> {
    fn new(
        iface_gen: &'a mut InterfaceGenerator<'b>,
        func: &'a Function,
        variant: AbiVariant,
        lift_lower: LiftLower,
        call_target: String,
    ) -> Self {
        Self {
            iface_gen,
            func,
            variant,
            lift_lower,
            call_target,
            next_local: 0,
            local_prefix: "v",
            after_call: String::new(),
            after_wasm_call: String::new(),
            async_export_arg_count: 0,
            skip_endpoint_drops: false,
            next_endpoint: 0,
            src: String::new(),
            blocks: Vec::new(),
            block_storage: Vec::new(),
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_local;
        self.next_local += 1;
        format!("{}{n}", self.local_prefix)
    }

    fn into_body(self) -> String {
        self.src
    }

    /// Queue the release of a buffer that was allocated purely to hand a
    /// synchronous import a borrowed view of a list. Runs after `CallWasm`.
    fn free_borrowed_list(
        &mut self,
        buf: &str,
        len: &str,
        element: &Type,
        size: usize,
        align: usize,
    ) {
        // Generate the per-element deallocation into a scratch buffer so it can
        // be replayed after the call rather than before it. `fresh()` keeps
        // counting, so the names stay unique in the spliced body.
        const BASE: &str = "__CLEANUP_BASE__";
        let outer = mem::take(&mut self.src);
        abi::deallocate_lists_in_types(
            self.iface_gen.resolve,
            &[*element],
            &[BASE.to_string()],
            true,
            self,
        );
        let element_cleanup = mem::replace(&mut self.src, outer);

        let mut cleanup = String::new();
        if !element_cleanup.trim().is_empty() {
            let i = self.fresh();
            let base = self.fresh();
            writeln!(cleanup, "for (let {i}: i32 = 0; {i} < {len}; {i}++) {{").unwrap();
            writeln!(
                cleanup,
                "  const {base}: usize = {buf} + <usize>({i} * {size});"
            )
            .unwrap();
            for line in element_cleanup.lines() {
                writeln!(cleanup, "  {}", line.replace(BASE, &base)).unwrap();
            }
            writeln!(cleanup, "}}").unwrap();
        }
        writeln!(
            cleanup,
            "ffi.cabi_realloc({buf}, <usize>({len} * {size}), {align}, 0);"
        )
        .unwrap();
        self.after_wasm_call.push_str(&cleanup);
    }

    /// Emit the per-element deallocation loop shared by list and map
    /// deallocation. `block_src` is an `abi` block whose `IterBasePointer`
    /// placeholder is substituted with each element's address. An empty block
    /// means the element itself owns nothing, so no loop is emitted.
    fn emit_element_dealloc_loop(&mut self, block_src: &str, ptr: &str, len: &str, size: usize) {
        if block_src.trim().is_empty() {
            return;
        }
        let i = self.fresh();
        let base = self.fresh();
        self.push_line(&format!("for (let {i}: i32 = 0; {i} < {len}; {i}++) {{"));
        self.push_line(&format!(
            "  const {base}: usize = {ptr} + <usize>({i} * {size});"
        ));
        for line in block_src.lines() {
            let l = line.replace("__ITER_BASE__", &base);
            self.push_line(&format!("  {l}"));
        }
        self.push_line("}");
    }

    fn push_line(&mut self, line: &str) {
        self.src.push_str(line);
        self.src.push('\n');
    }
}

impl<'a, 'b> Bindgen for FunctionBindgen<'a, 'b> {
    type Operand = String;

    fn emit(
        &mut self,
        _resolve: &Resolve,
        inst: &Instruction<'_>,
        operands: &mut Vec<Self::Operand>,
        results: &mut Vec<Self::Operand>,
    ) {
        match inst {
            Instruction::GetArg { nth } => {
                let name = match self.lift_lower {
                    LiftLower::LowerArgsLiftResults => {
                        // Lower side: AS user args are named per the function signature.
                        let pname = ident::value_name(&self.func.params[*nth].name);
                        pname
                    }
                    LiftLower::LiftArgsLowerResults => {
                        // Lift side: incoming wasm args are named a0, a1, ...
                        format!("a{nth}")
                    }
                };
                results.push(name);
            }

            Instruction::I32Const { val } => results.push(format!("(<i32>{val})")),
            Instruction::ConstZero { tys } => {
                for ty in *tys {
                    results.push(match ty {
                        WasmType::I32 => "(<i32>0)".into(),
                        WasmType::I64 => "(<i64>0)".into(),
                        WasmType::F32 => "(<f32>0.0)".into(),
                        WasmType::F64 => "(<f64>0.0)".into(),
                        WasmType::Pointer => "(<usize>0)".into(),
                        WasmType::Length => "(<usize>0)".into(),
                        WasmType::PointerOrI64 => "(<i64>0)".into(),
                    });
                }
            }

            Instruction::Bitcasts { casts } => {
                for (cast, op) in casts.iter().zip(operands.drain(..)) {
                    results.push(bitcast_expr(cast, &op));
                }
            }

            // Memory loads
            Instruction::I32Load { offset } => {
                results.push(format!(
                    "load<i32>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::I32Load8U { offset } => {
                results.push(format!(
                    "(<i32>load<u8>({} + {}))",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::I32Load8S { offset } => {
                results.push(format!(
                    "(<i32>load<i8>({} + {}))",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::I32Load16U { offset } => {
                results.push(format!(
                    "(<i32>load<u16>({} + {}))",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::I32Load16S { offset } => {
                results.push(format!(
                    "(<i32>load<i16>({} + {}))",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::I64Load { offset } => {
                results.push(format!(
                    "load<i64>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::F32Load { offset } => {
                results.push(format!(
                    "load<f32>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::F64Load { offset } => {
                results.push(format!(
                    "load<f64>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::PointerLoad { offset } => {
                results.push(format!(
                    "load<usize>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }
            Instruction::LengthLoad { offset } => {
                results.push(format!(
                    "load<usize>({} + {})",
                    operands[0],
                    offset.size_wasm32()
                ));
            }

            // Memory stores
            Instruction::I32Store { offset } => {
                self.push_line(&format!(
                    "store<i32>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::I32Store8 { offset } => {
                self.push_line(&format!(
                    "store<u8>({} + {}, <u8>({} & 0xff));",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::I32Store16 { offset } => {
                self.push_line(&format!(
                    "store<u16>({} + {}, <u16>({} & 0xffff));",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::I64Store { offset } => {
                self.push_line(&format!(
                    "store<i64>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::F32Store { offset } => {
                self.push_line(&format!(
                    "store<f32>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::F64Store { offset } => {
                self.push_line(&format!(
                    "store<f64>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::PointerStore { offset } => {
                self.push_line(&format!(
                    "store<usize>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }
            Instruction::LengthStore { offset } => {
                self.push_line(&format!(
                    "store<usize>({} + {}, {});",
                    operands[1],
                    offset.size_wasm32(),
                    operands[0]
                ));
            }

            // Scalar lowerings
            Instruction::I32FromChar | Instruction::I32FromU32 | Instruction::I32FromS32 => {
                results.push(format!("(<i32>{})", operands[0]))
            }
            Instruction::I32FromU8 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::I32FromS8 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::I32FromU16 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::I32FromS16 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::I64FromU64 | Instruction::I64FromS64 => {
                results.push(format!("(<i64>{})", operands[0]));
            }
            Instruction::CoreF32FromF32 => results.push(format!("(<f32>{})", operands[0])),
            Instruction::CoreF64FromF64 => results.push(format!("(<f64>{})", operands[0])),

            // Scalar liftings
            Instruction::S8FromI32 => results.push(format!("(<i8>{})", operands[0])),
            Instruction::U8FromI32 => results.push(format!("(<u8>{})", operands[0])),
            Instruction::S16FromI32 => results.push(format!("(<i16>{})", operands[0])),
            Instruction::U16FromI32 => results.push(format!("(<u16>{})", operands[0])),
            Instruction::S32FromI32 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::U32FromI32 => results.push(format!("(<u32>{})", operands[0])),
            Instruction::S64FromI64 => results.push(format!("(<i64>{})", operands[0])),
            Instruction::U64FromI64 => results.push(format!("(<u64>{})", operands[0])),
            Instruction::CharFromI32 => results.push(format!("(<i32>{})", operands[0])),
            Instruction::F32FromCoreF32 => results.push(format!("(<f32>{})", operands[0])),
            Instruction::F64FromCoreF64 => results.push(format!("(<f64>{})", operands[0])),

            Instruction::BoolFromI32 => results.push(format!("({} != 0)", operands[0])),
            Instruction::I32FromBool => results.push(format!("({} ? 1 : 0)", operands[0])),

            // Strings
            Instruction::StringLower { realloc } => {
                let s_ref = self.fresh();
                let s_ptr = self.fresh();
                let s_len = self.fresh();
                self.push_line(&format!("const {s_ref} = {};", operands[0]));
                if realloc.is_some() {
                    // Need to copy the string into a wasm-owned buffer.
                    let bytes = self.fresh();
                    let buf = self.fresh();
                    self.push_line(&format!("const {bytes} = (<usize>{s_ref}.length) << 1;"));
                    self.push_line(&format!(
                        "const {buf} = ffi.cabi_realloc(0, 0, 2, {bytes});"
                    ));
                    self.push_line(&format!(
                        "memory.copy({buf}, changetype<usize>({s_ref}), {bytes});"
                    ));
                    self.push_line(&format!("const {s_ptr} = {buf};"));
                    self.push_line(&format!("const {s_len} = <i32>{s_ref}.length;"));
                } else {
                    self.push_line(&format!("const {s_ptr} = changetype<usize>({s_ref});"));
                    self.push_line(&format!("const {s_len} = <i32>{s_ref}.length;"));
                }
                results.push(s_ptr);
                results.push(s_len);
            }

            Instruction::StringLift => {
                results.push(format!(
                    "ffi.strLift(<usize>{}, <usize>{})",
                    operands[0], operands[1]
                ));
            }

            // Canonical numeric lists
            Instruction::ListCanonLower { element, realloc } => {
                let arr_ref = self.fresh();
                self.push_line(&format!("const {arr_ref} = {};", operands[0]));
                let helper = numeric_array_lower(element);
                if let Some(helper) = helper {
                    let ptr = self.fresh();
                    let len = self.fresh();
                    if realloc.is_some() {
                        // The callee takes ownership, so hand it a copy.
                        self.push_line(&format!("const {ptr} = ffi.{helper}({arr_ref});"));
                    } else {
                        // `Realloc::None` means the callee only borrows for the
                        // duration of the call, and nothing would ever free a
                        // copy. Point at the array's own storage instead;
                        // `arr_ref` is a named local, so it stays a GC root
                        // across the call. Same discipline as `StringLower`.
                        self.push_line(&format!(
                            "const {ptr} = changetype<usize>({arr_ref}.dataStart);"
                        ));
                    }
                    self.push_line(&format!("const {len} = <i32>{arr_ref}.length;"));
                    results.push(ptr);
                    results.push(len);
                } else {
                    let ptr = self.fresh();
                    let len = self.fresh();
                    self.push_line(&format!(
                        "const {ptr} = changetype<usize>({arr_ref}.dataStart);"
                    ));
                    self.push_line(&format!("const {len} = <i32>{arr_ref}.length;"));
                    results.push(ptr);
                    results.push(len);
                }
            }

            Instruction::ListCanonLift { element, .. } => {
                let helper = numeric_array_lift(element);
                if let Some(helper) = helper {
                    results.push(format!(
                        "ffi.{helper}(<usize>{}, <u32>{})",
                        operands[0], operands[1]
                    ));
                } else {
                    // Non-numeric canonical list — fallback view.
                    results.push(format!(
                        "ffi.u8ArrayLift(<usize>{}, <u32>{}) /* canonical fallback */",
                        operands[0], operands[1]
                    ));
                }
            }

            // Non-canonical lists: use the block-stack pattern.
            Instruction::ListLower { element, realloc } => {
                let arr_ref = self.fresh();
                let buf = self.fresh();
                let len = self.fresh();
                let size = self.iface_gen.world_gen.sizes.size(element).size_wasm32();
                let align = self.iface_gen.world_gen.sizes.align(element);
                self.push_line(&format!("const {arr_ref} = {};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{arr_ref}.length;"));
                self.push_line(&format!(
                    "const {buf} = ffi.cabi_realloc(0, 0, {}, <usize>({len} * {size}));",
                    align.align_wasm32()
                ));
                let (block_src, block_results) =
                    self.blocks.pop().expect("ListLower expects a block");
                // `Realloc::None` means the callee only borrows this buffer, so
                // the caller has to release it — including any list or string
                // an element lowered into linear memory.
                //
                // Only at the top level: `buf`/`len` for a nested list are
                // loop-locals that no longer exist after the call, and the
                // enclosing list's own cleanup already walks into its elements.
                // A list nested inside a record or variant parameter is still
                // not released.
                if realloc.is_none() && self.block_storage.is_empty() {
                    self.free_borrowed_list(&buf, &len, element, size, align.align_wasm32());
                }
                let i = self.fresh();
                let base = self.fresh();
                let elem = self.fresh();
                self.push_line(&format!("for (let {i}: i32 = 0; {i} < {len}; {i}++) {{"));
                self.push_line(&format!("  const {elem} = {arr_ref}[{i}];"));
                self.push_line(&format!(
                    "  const {base}: usize = {buf} + <usize>({i} * {size});"
                ));
                // The block references `IterBasePointer` (-> we passed `base`) and
                // `IterElem` (-> `elem`). The block source uses ${0}/${1} style
                // identifiers we filled via emit. We approximate by inserting
                // a placeholder substitution here.
                for line in block_src.lines() {
                    let l = line
                        .replace("__ITER_BASE__", &base)
                        .replace("__ITER_ELEM__", &elem);
                    self.push_line(&format!("  {l}"));
                }
                let _ = block_results;
                self.push_line("}");
                results.push(buf);
                results.push(len);
            }

            Instruction::ListLift { element, .. } => {
                let ptr = self.fresh();
                let len = self.fresh();
                let arr = self.fresh();
                self.push_line(&format!("const {ptr} = <usize>{};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{};", operands[1]));
                let elem_ty = self.iface_gen.type_ref(element);
                self.push_line(&format!("const {arr} = new Array<{elem_ty}>({len});"));
                let (block_src, block_results) =
                    self.blocks.pop().expect("ListLift expects a block");
                let i = self.fresh();
                let base = self.fresh();
                let size = self.iface_gen.world_gen.sizes.size(element).size_wasm32();
                self.push_line(&format!("for (let {i}: i32 = 0; {i} < {len}; {i}++) {{"));
                self.push_line(&format!(
                    "  const {base}: usize = {ptr} + <usize>({i} * {size});"
                ));
                for line in block_src.lines() {
                    let l = line.replace("__ITER_BASE__", &base);
                    self.push_line(&format!("  {l}"));
                }
                if let Some(r) = block_results.into_iter().next() {
                    let r = r.replace("__ITER_BASE__", &base);
                    self.push_line(&format!("  {arr}[{i}] = {r};"));
                }
                self.push_line("}");
                results.push(arr);
            }

            Instruction::IterElem { .. } => results.push("__ITER_ELEM__".into()),
            Instruction::IterBasePointer => results.push("__ITER_BASE__".into()),
            Instruction::IterMapKey { .. } => results.push("__ITER_MAP_KEY__".into()),
            Instruction::IterMapValue { .. } => results.push("__ITER_MAP_VALUE__".into()),

            // Records
            Instruction::RecordLower { record, .. } => {
                let r_ref = self.fresh();
                self.push_line(&format!("const {r_ref} = {};", operands[0]));
                for field in record.fields.iter() {
                    let f = InterfaceGenerator::field_ident(&field.name);
                    results.push(format!("{r_ref}.{f}"));
                }
            }

            Instruction::RecordLift { ty, .. } => {
                let cls = self.iface_gen.named_type_ref(*ty);
                let args = operands.drain(..).collect::<Vec<_>>().join(", ");
                results.push(format!("new {cls}({args})"));
            }

            // Tuples — represented as ffi.TupleN<...> class instances.
            Instruction::TupleLower { tuple, .. } => {
                let t_ref = self.fresh();
                self.push_line(&format!("const {t_ref} = {};", operands[0]));
                for i in 0..tuple.types.len() {
                    results.push(format!("{t_ref}._{i}"));
                }
            }
            Instruction::TupleLift { tuple, .. } => {
                let arity = tuple.types.len();
                let args = operands.drain(..).collect::<Vec<_>>().join(", ");
                results.push(format!("new ffi.Tuple{arity}({args})"));
            }

            // Handles (resources). The lowering depends on whether the
            // resource is exported (we own; use handle-table) or imported (we
            // hold the wrapper class with a `.handle` field).
            Instruction::HandleLower { handle, .. } => {
                let raw_id = match handle {
                    Handle::Own(id) | Handle::Borrow(id) => *id,
                };
                let res_id = wit_bindgen_core::dealias(self.iface_gen.resolve, raw_id);
                if self
                    .iface_gen
                    .world_gen
                    .exported_resources
                    .contains(&res_id)
                {
                    let prefix = resource_table_prefix(self.iface_gen, res_id);
                    let ty = &self.iface_gen.resolve.types[res_id];
                    let res_name = ty.name.clone().unwrap_or_default();
                    let safe = sanitize_extern_local(&ident::type_name(&res_name));
                    results.push(format!("{prefix}__{safe}_take({})", operands[0]));
                } else {
                    results.push(format!("{}.handle", operands[0]));
                }
            }
            Instruction::HandleLift { handle, .. } => {
                let raw_id = match handle {
                    Handle::Own(id) | Handle::Borrow(id) => *id,
                };
                let res_id = wit_bindgen_core::dealias(self.iface_gen.resolve, raw_id);
                if self
                    .iface_gen
                    .world_gen
                    .exported_resources
                    .contains(&res_id)
                {
                    let prefix = resource_table_prefix(self.iface_gen, res_id);
                    let ty = &self.iface_gen.resolve.types[res_id];
                    let res_name = ty.name.clone().unwrap_or_default();
                    let safe = sanitize_extern_local(&ident::type_name(&res_name));
                    results.push(format!("{prefix}__{safe}_get({})", operands[0]));
                } else {
                    let cls = self.iface_gen.named_type_ref(res_id);
                    results.push(format!("new {cls}({})", operands[0]));
                }
            }

            // Futures and streams remain opaque handles. Error contexts are
            // owned values, so keep the typed wrapper at the source level.
            Instruction::FutureLower { .. } | Instruction::StreamLower { .. } => {
                results.push(format!("(<i32>{})", operands[0]));
            }
            Instruction::ErrorContextLower => {
                results.push(format!("{}.handle", operands[0]));
            }
            Instruction::FutureLift { .. } | Instruction::StreamLift { .. } => {
                results.push(format!("(<i32>{})", operands[0]));
            }
            Instruction::ErrorContextLift => {
                results.push(format!("new async_.ErrorContext(<i32>{})", operands[0]));
            }

            // Flags
            Instruction::FlagsLower { flags, .. } => {
                let bits_ref = self.fresh();
                self.push_line(&format!("const {bits_ref} = {}.bits;", operands[0]));
                match flags.repr() {
                    FlagsRepr::U8 | FlagsRepr::U16 | FlagsRepr::U32(1) => {
                        results.push(format!("(<i32>{bits_ref})"));
                    }
                    FlagsRepr::U32(2) => {
                        results.push(format!("(<i32>({bits_ref} & 0xffffffff))"));
                        results.push(format!("(<i32>({bits_ref} >> 32))"));
                    }
                    _ => panic!("unsupported flags repr"),
                }
            }
            Instruction::FlagsLift { flags, ty, .. } => {
                let cls = self.iface_gen.named_type_ref(*ty);
                let cast = match flags.repr() {
                    FlagsRepr::U8 => "<u8>",
                    FlagsRepr::U16 => "<u16>",
                    FlagsRepr::U32(1) => "<u32>",
                    FlagsRepr::U32(2) => "",
                    _ => panic!("unsupported flags repr"),
                };
                match flags.repr() {
                    FlagsRepr::U8 | FlagsRepr::U16 | FlagsRepr::U32(1) => {
                        results.push(format!("new {cls}({cast}{})", operands[0]));
                    }
                    FlagsRepr::U32(2) => {
                        results.push(format!(
                            "new {cls}((<u64>{}) | ((<u64>{}) << 32))",
                            operands[0], operands[1]
                        ));
                    }
                    _ => panic!("unsupported flags repr"),
                }
            }

            // Variants
            Instruction::VariantPayloadName => results.push("__variant_payload__".into()),

            Instruction::VariantLower {
                variant,
                results: wasm_results,
                ty,
                ..
            } => {
                let v_ref = self.fresh();
                self.push_line(&format!("const {v_ref} = {};", operands[0]));
                let cls = self.iface_gen.named_type_ref(*ty);
                let result_locals: Vec<String> =
                    wasm_results.iter().map(|_| self.fresh()).collect();
                for (loc, ty) in result_locals.iter().zip(wasm_results.iter()) {
                    self.push_line(&format!("let {loc}: {} = 0;", wasm_type_name(*ty)));
                }
                // Pop all blocks (LIFO) then reverse so case 0 -> first block.
                let mut popped: Vec<_> = (0..variant.cases.len())
                    .map(|_| self.blocks.pop().expect("VariantLower expects a block"))
                    .collect();
                popped.reverse();
                self.push_line(&format!("switch ({v_ref}.tag) {{"));
                for (i, (case, (block_src, block_results))) in
                    variant.cases.iter().zip(popped.into_iter()).enumerate()
                {
                    self.push_line(&format!("  case {i}: {{"));
                    let cname = ident::case_name(&case.name);
                    if case.ty.is_some() {
                        self.push_line(&format!(
                            "    const __variant_payload__ = (({v_ref} as {cls}_{cname})).value;"
                        ));
                    }
                    for line in block_src.lines() {
                        self.push_line(&format!("    {line}"));
                    }
                    for (loc, br) in result_locals.iter().zip(block_results.iter()) {
                        self.push_line(&format!("    {loc} = {br};"));
                    }
                    self.push_line("    break;");
                    self.push_line("  }");
                }
                self.push_line("}");
                results.extend(result_locals);
            }

            Instruction::VariantLift { variant, ty, .. } => {
                let tag_ref = self.fresh();
                let out = self.fresh();
                let cls = self.iface_gen.named_type_ref(*ty);
                self.push_line(&format!("const {tag_ref} = {};", operands[0]));
                self.push_line(&format!("let {out}: {cls} = changetype<{cls}>(0);"));
                let mut popped: Vec<_> = (0..variant.cases.len())
                    .map(|_| self.blocks.pop().expect("VariantLift expects a block"))
                    .collect();
                popped.reverse();
                self.push_line(&format!("switch ({tag_ref}) {{"));
                for (i, (case, (block_src, block_results))) in
                    variant.cases.iter().zip(popped.into_iter()).enumerate()
                {
                    self.push_line(&format!("  case {i}: {{"));
                    for line in block_src.lines() {
                        self.push_line(&format!("    {line}"));
                    }
                    let cname = ident::case_name(&case.name);
                    let arg = block_results.into_iter().next().unwrap_or_default();
                    if case.ty.is_some() {
                        self.push_line(&format!("    {out} = new {cls}_{cname}({arg});"));
                    } else {
                        self.push_line(&format!("    {out} = new {cls}_{cname}();"));
                    }
                    self.push_line("    break;");
                    self.push_line("  }");
                }
                self.push_line("}");
                results.push(out);
            }

            Instruction::EnumLower { .. } => {
                results.push(format!("(<i32>{})", operands[0]));
            }
            Instruction::EnumLift { ty, .. } => {
                let cls = self.iface_gen.named_type_ref(*ty);
                results.push(format!("<{cls}>{}", operands[0]));
            }

            Instruction::OptionLower {
                payload,
                results: wasm_results,
                ..
            } => {
                let o_ref = self.fresh();
                self.push_line(&format!("const {o_ref} = {};", operands[0]));
                let result_locals: Vec<String> =
                    wasm_results.iter().map(|_| self.fresh()).collect();
                for (loc, ty) in result_locals.iter().zip(wasm_results.iter()) {
                    self.push_line(&format!("let {loc}: {} = 0;", wasm_type_name(*ty)));
                }
                let (some_src, some_res) =
                    self.blocks.pop().expect("OptionLower expects Some block");
                let (none_src, none_res) =
                    self.blocks.pop().expect("OptionLower expects None block");
                let _ = payload;
                self.push_line(&format!("if ({o_ref}.tag == 1) {{"));
                self.push_line(&format!("  const __variant_payload__ = {o_ref}.value;"));
                for line in some_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                for (loc, br) in result_locals.iter().zip(some_res.iter()) {
                    self.push_line(&format!("  {loc} = {br};"));
                }
                self.push_line("} else {");
                for line in none_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                for (loc, br) in result_locals.iter().zip(none_res.iter()) {
                    self.push_line(&format!("  {loc} = {br};"));
                }
                self.push_line("}");
                results.extend(result_locals);
            }

            Instruction::OptionLift { payload, .. } => {
                let tag = self.fresh();
                let out = self.fresh();
                let payload_ty = self.iface_gen.type_ref(payload);
                let default = default_value_for(&payload_ty);
                self.push_line(&format!("const {tag} = {};", operands[0]));
                self.push_line(&format!(
                    "let {out}: ffi.Option<{payload_ty}> = new ffi.Option<{payload_ty}>(0, {default});"
                ));
                let (some_src, some_res) = self.blocks.pop().expect("OptionLift Some");
                let (none_src, none_res) = self.blocks.pop().expect("OptionLift None");
                let _ = none_res;
                self.push_line(&format!("if ({tag} == 1) {{"));
                for line in some_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                if let Some(r) = some_res.into_iter().next() {
                    self.push_line(&format!("  {out} = new ffi.Option<{payload_ty}>(1, {r});"));
                }
                self.push_line("} else {");
                for line in none_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                self.push_line("}");
                results.push(out);
            }

            Instruction::ResultLower {
                result: r,
                results: wasm_results,
                ..
            } => {
                let r_ref = self.fresh();
                self.push_line(&format!("const {r_ref} = {};", operands[0]));
                let result_locals: Vec<String> =
                    wasm_results.iter().map(|_| self.fresh()).collect();
                for (loc, ty) in result_locals.iter().zip(wasm_results.iter()) {
                    self.push_line(&format!("let {loc}: {} = 0;", wasm_type_name(*ty)));
                }
                let (err_src, err_res) = self.blocks.pop().expect("ResultLower Err");
                let (ok_src, ok_res) = self.blocks.pop().expect("ResultLower Ok");
                self.push_line(&format!("if ({r_ref}.tag == 0) {{"));
                if r.ok.is_some() {
                    self.push_line(&format!("  const __variant_payload__ = {r_ref}.okValue;"));
                }
                for line in ok_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                for (loc, br) in result_locals.iter().zip(ok_res.iter()) {
                    self.push_line(&format!("  {loc} = {br};"));
                }
                self.push_line("} else {");
                if r.err.is_some() {
                    self.push_line(&format!("  const __variant_payload__ = {r_ref}.errValue;"));
                }
                for line in err_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                for (loc, br) in result_locals.iter().zip(err_res.iter()) {
                    self.push_line(&format!("  {loc} = {br};"));
                }
                self.push_line("}");
                results.extend(result_locals);
            }

            Instruction::ResultLift { result: r, .. } => {
                let tag = self.fresh();
                let out = self.fresh();
                let ok_ty =
                    r.ok.map(|t| self.iface_gen.type_ref(&t))
                        .unwrap_or_else(|| "i32".into());
                let err_ty = r
                    .err
                    .map(|t| self.iface_gen.type_ref(&t))
                    .unwrap_or_else(|| "i32".into());
                let ok_default = default_value_for(&ok_ty);
                let err_default = default_value_for(&err_ty);
                self.push_line(&format!("const {tag} = {};", operands[0]));
                let (err_src, err_res) = self.blocks.pop().expect("ResultLift Err");
                let (ok_src, ok_res) = self.blocks.pop().expect("ResultLift Ok");
                self.push_line(&format!(
                    "let {out}: ffi.Result<{ok_ty}, {err_ty}> = new ffi.Result<{ok_ty}, {err_ty}>(0, {ok_default}, {err_default});"
                ));
                self.push_line(&format!("if ({tag} == 0) {{"));
                for line in ok_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                let ok_arg = ok_res
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| ok_default.clone());
                self.push_line(&format!(
                    "  {out} = new ffi.Result<{ok_ty}, {err_ty}>(0, {ok_arg}, {err_default});"
                ));
                self.push_line("} else {");
                for line in err_src.lines() {
                    self.push_line(&format!("  {line}"));
                }
                let err_arg = err_res
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| err_default.clone());
                self.push_line(&format!(
                    "  {out} = new ffi.Result<{ok_ty}, {err_ty}>(1, {ok_default}, {err_arg});"
                ));
                self.push_line("}");
                results.push(out);
            }

            // Fixed-length lists — emit straightforward unrolled or loop forms.
            Instruction::FixedLengthListLower { size, .. } => {
                let r = self.fresh();
                self.push_line(&format!("const {r} = {};", operands[0]));
                for i in 0..*size {
                    results.push(format!("{r}[{i}]"));
                }
            }
            Instruction::FixedLengthListLift { size, element, .. } => {
                let t = self.iface_gen.type_ref(element);
                let arr = self.fresh();
                self.push_line(&format!("const {arr} = new StaticArray<{t}>({size});"));
                for (i, op) in operands.drain(..).enumerate() {
                    self.push_line(&format!("{arr}[{i}] = {op};"));
                }
                results.push(arr);
            }
            Instruction::FixedLengthListLowerToMemory { size, element, .. } => {
                let arr = self.fresh();
                let addr = self.fresh();
                let elem_size = self.iface_gen.world_gen.sizes.size(element).size_wasm32();
                self.push_line(&format!("const {arr} = {};", operands[0]));
                self.push_line(&format!("const {addr} = {};", operands[1]));
                let (block_src, _) = self
                    .blocks
                    .pop()
                    .expect("FixedLengthListLowerToMemory expects a block");
                for i in 0..*size {
                    let base = self.fresh();
                    let elem = self.fresh();
                    self.push_line(&format!(
                        "const {base}: usize = {addr} + <usize>({i} * {elem_size});"
                    ));
                    self.push_line(&format!("const {elem} = {arr}[{i}];"));
                    for line in block_src.lines() {
                        let l = line
                            .replace("__ITER_BASE__", &base)
                            .replace("__ITER_ELEM__", &elem);
                        self.push_line(&l);
                    }
                }
            }
            Instruction::FixedLengthListLiftFromMemory { size, element, .. } => {
                let addr = self.fresh();
                let arr = self.fresh();
                let t = self.iface_gen.type_ref(element);
                let elem_size = self.iface_gen.world_gen.sizes.size(element).size_wasm32();
                self.push_line(&format!("const {addr} = {};", operands[0]));
                self.push_line(&format!("const {arr} = new StaticArray<{t}>({size});"));
                let (block_src, block_results) = self
                    .blocks
                    .pop()
                    .expect("FixedLengthListLiftFromMemory expects a block");
                for i in 0..*size {
                    let base = self.fresh();
                    self.push_line(&format!(
                        "const {base}: usize = {addr} + <usize>({i} * {elem_size});"
                    ));
                    for line in block_src.lines() {
                        let l = line.replace("__ITER_BASE__", &base);
                        self.push_line(&l);
                    }
                    if let Some(r) = block_results.first() {
                        let r = r.replace("__ITER_BASE__", &base);
                        self.push_line(&format!("{arr}[{i}] = {r};"));
                    }
                }
                results.push(arr);
            }

            // Maps — canonical-ABI layout: a list of (key, value) entries, each
            // entry sized like `record { key, value }`.
            Instruction::MapLower { key, value, .. } => {
                let entry = self.iface_gen.world_gen.sizes.record([*key, *value]);
                let entry_size = entry.size.size_wasm32();
                let entry_align = entry.align.align_wasm32();
                let map_ref = self.fresh();
                let len = self.fresh();
                let buf = self.fresh();
                self.push_line(&format!("const {map_ref} = {};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{map_ref}.size;"));
                self.push_line(&format!(
                    "const {buf} = ffi.cabi_realloc(0, 0, {entry_align}, <usize>({len} * {entry_size}));"
                ));
                let (block_src, _) = self.blocks.pop().expect("MapLower expects a block");
                let i = self.fresh();
                let keys = self.fresh();
                let k = self.fresh();
                let v = self.fresh();
                let base = self.fresh();
                self.push_line(&format!("const {keys} = {map_ref}.keys();"));
                self.push_line(&format!("for (let {i}: i32 = 0; {i} < {len}; {i}++) {{"));
                self.push_line(&format!("  const {k} = {keys}[{i}];"));
                self.push_line(&format!("  const {v} = {map_ref}.get({k});"));
                self.push_line(&format!(
                    "  const {base}: usize = {buf} + <usize>({i} * {entry_size});"
                ));
                for line in block_src.lines() {
                    let l = line
                        .replace("__ITER_BASE__", &base)
                        .replace("__ITER_MAP_KEY__", &k)
                        .replace("__ITER_MAP_VALUE__", &v);
                    self.push_line(&format!("  {l}"));
                }
                self.push_line("}");
                results.push(buf);
                results.push(len);
            }

            Instruction::MapLift { key, value, .. } => {
                let entry = self.iface_gen.world_gen.sizes.record([*key, *value]);
                let entry_size = entry.size.size_wasm32();
                let ptr = self.fresh();
                let len = self.fresh();
                let map_local = self.fresh();
                let key_ty = self.iface_gen.type_ref(key);
                let val_ty = self.iface_gen.type_ref(value);
                self.push_line(&format!("const {ptr} = <usize>{};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{};", operands[1]));
                self.push_line(&format!(
                    "const {map_local} = new Map<{key_ty}, {val_ty}>();"
                ));
                let (block_src, block_results) =
                    self.blocks.pop().expect("MapLift expects a block");
                let i = self.fresh();
                let base = self.fresh();
                self.push_line(&format!("for (let {i}: i32 = 0; {i} < {len}; {i}++) {{"));
                self.push_line(&format!(
                    "  const {base}: usize = {ptr} + <usize>({i} * {entry_size});"
                ));
                for line in block_src.lines() {
                    let l = line.replace("__ITER_BASE__", &base);
                    self.push_line(&format!("  {l}"));
                }
                if let [k, v] = &block_results[..] {
                    let k = k.replace("__ITER_BASE__", &base);
                    let v = v.replace("__ITER_BASE__", &base);
                    self.push_line(&format!("  {map_local}.set({k}, {v});"));
                }
                self.push_line("}");
                results.push(map_local);
            }

            // Calls
            Instruction::CallWasm { name, sig } => {
                let _ = name;
                let args = operands.drain(..).collect::<Vec<_>>().join(", ");
                if sig.results.is_empty() {
                    self.push_line(&format!("{}({args});", self.call_target));
                } else if sig.results.len() == 1 {
                    let r = self.fresh();
                    self.push_line(&format!("const {r} = {}({args});", self.call_target));
                    results.push(r);
                } else {
                    // Multi-return: result lives in the return area; the caller
                    // will issue `*Load` ops with the area pointer.
                    self.push_line(&format!("{}({args});", self.call_target));
                }
                let cleanup = mem::take(&mut self.after_wasm_call);
                for line in cleanup.lines() {
                    self.push_line(line);
                }
            }

            Instruction::CallInterface { func, async_ } => {
                let args = operands.drain(..).collect::<Vec<_>>().join(", ");
                if *async_ {
                    let task = self.fresh();
                    self.push_line(&format!("const {task} = {}({args});", self.call_target));
                    for i in 0..self.async_export_arg_count {
                        self.push_line(&format!("{task}.__arg{i} = a{i};"));
                    }
                    self.push_line(&format!("return async_.Scheduler.start({task});"));
                    if let Some(result) = &func.result {
                        let ty = self.iface_gen.type_ref(result);
                        results.push(default_value_for(&ty));
                    }
                    return;
                }
                if func.result.is_some() {
                    let r = self.fresh();
                    self.push_line(&format!("const {r} = {}({args});", self.call_target));
                    results.push(r);
                } else {
                    self.push_line(&format!("{}({args});", self.call_target));
                }
                let cleanup = mem::take(&mut self.after_call);
                for line in cleanup.lines() {
                    self.push_line(line);
                }
            }

            Instruction::Return { amt, .. } => match amt {
                0 => self.push_line("return;"),
                1 => self.push_line(&format!("return {};", operands[0])),
                _ => self.push_line(&format!("return [{}];", operands.join(", "))),
            },

            Instruction::Malloc { size, align, .. } => {
                results.push(format!(
                    "ffi.cabi_realloc(0, 0, {}, {})",
                    align.align_wasm32(),
                    size.size_wasm32()
                ));
            }

            Instruction::GuestDeallocate { size, align } => {
                self.push_line(&format!(
                    "ffi.cabi_realloc({}, {}, {}, 0);",
                    operands[0],
                    size.size_wasm32(),
                    align.align_wasm32()
                ));
            }
            Instruction::GuestDeallocateString => {
                self.push_line(&format!(
                    "ffi.cabi_realloc({}, <usize>({} << 1), 2, 0);",
                    operands[0], operands[1]
                ));
            }
            Instruction::GuestDeallocateList { element } => {
                let (block_src, _) = self
                    .blocks
                    .pop()
                    .expect("GuestDeallocateList expects a block");
                let size = self.iface_gen.world_gen.sizes.size(element).size_wasm32();
                let align = self.iface_gen.world_gen.sizes.align(element).align_wasm32();
                let ptr = self.fresh();
                let len = self.fresh();
                self.push_line(&format!("const {ptr} = <usize>{};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{};", operands[1]));
                self.emit_element_dealloc_loop(&block_src, &ptr, &len, size);
                // The buffer was allocated with `len * elem_size` bytes, not
                // `len` bytes.
                self.push_line(&format!(
                    "ffi.cabi_realloc({ptr}, <usize>({len} * {size}), {align}, 0);"
                ));
            }
            Instruction::GuestDeallocateMap { key, value } => {
                let (block_src, _) = self
                    .blocks
                    .pop()
                    .expect("GuestDeallocateMap expects a block");
                let layout = self.iface_gen.world_gen.sizes.record([*key, *value]);
                let size = layout.size.size_wasm32();
                let align = layout.align.align_wasm32();
                let ptr = self.fresh();
                let len = self.fresh();
                self.push_line(&format!("const {ptr} = <usize>{};", operands[0]));
                self.push_line(&format!("const {len} = <i32>{};", operands[1]));
                self.emit_element_dealloc_loop(&block_src, &ptr, &len, size);
                self.push_line(&format!(
                    "ffi.cabi_realloc({ptr}, <usize>({len} * {size}), {align}, 0);"
                ));
            }
            Instruction::GuestDeallocateVariant { blocks } => {
                // Pop all blocks (LIFO) then reverse so case 0 -> first block.
                let mut cases = (0..*blocks)
                    .map(|_| {
                        self.blocks
                            .pop()
                            .expect("GuestDeallocateVariant expects a block")
                    })
                    .collect::<Vec<_>>();
                cases.reverse();
                // Without the tag switch the active payload — and any owned
                // handle inside it — is never released.
                self.push_line(&format!("switch (<i32>{}) {{", operands[0]));
                let last = cases.len() - 1;
                for (i, (block_src, _)) in cases.into_iter().enumerate() {
                    if i == last {
                        self.push_line("  default: {");
                    } else {
                        self.push_line(&format!("  case {i}: {{"));
                    }
                    for line in block_src.lines() {
                        self.push_line(&format!("    {line}"));
                    }
                    self.push_line("    break;");
                    self.push_line("  }");
                }
                self.push_line("}");
            }

            Instruction::DropHandle { ty } => {
                if matches!(ty, Type::ErrorContext) {
                    self.push_line(&format!("{}.drop();", operands[0]));
                    return;
                }
                if let Type::Id(id) = ty {
                    let id = wit_bindgen_core::dealias(self.iface_gen.resolve, *id);
                    // `abi::deallocate` lifts an owned handle before emitting
                    // `DropHandle`, so `operands[0]` is already the lifted value:
                    // the wrapper instance for an imported resource, or the
                    // exported class instance taken out of the resource table.
                    if let TypeDefKind::Handle(Handle::Own(resource_id)) =
                        &self.iface_gen.resolve.types[id].kind
                    {
                        let resource_id =
                            wit_bindgen_core::dealias(self.iface_gen.resolve, *resource_id);
                        if self
                            .iface_gen
                            .world_gen
                            .exported_resources
                            .contains(&resource_id)
                        {
                            let prefix = resource_table_prefix(self.iface_gen, resource_id);
                            let safe = sanitize_extern_local(&ident::type_name(
                                self.iface_gen.resolve.types[resource_id]
                                    .name
                                    .as_deref()
                                    .unwrap_or_default(),
                            ));
                            self.push_line(&format!(
                                "{prefix}__{safe}_drop_instance({});",
                                operands[0]
                            ));
                        } else {
                            self.push_line(&format!("{}.drop();", operands[0]));
                        }
                        return;
                    }
                }
                let Type::Id(id) = ty else {
                    let _ = operands;
                    return;
                };
                let id = wit_bindgen_core::dealias(self.iface_gen.resolve, *id);
                let kind = match &self.iface_gen.resolve.types[id].kind {
                    TypeDefKind::Future(_) => EndpointKind::Future,
                    TypeDefKind::Stream(_) => EndpointKind::Stream,
                    _ => {
                        let _ = operands;
                        return;
                    }
                };
                if self.skip_endpoint_drops {
                    return;
                }
                let endpoints = self.func.find_futures_and_streams(self.iface_gen.resolve);
                let index = endpoints
                    .iter()
                    .enumerate()
                    .skip(self.next_endpoint)
                    .find_map(|(index, endpoint)| {
                        (wit_bindgen_core::dealias(self.iface_gen.resolve, *endpoint) == id)
                            .then_some(index)
                    })
                    .expect("DropHandle has no matching function endpoint occurrence");
                self.next_endpoint = index + 1;
                let stem = format!(
                    "raw{}{}{}{index}",
                    if self.iface_gen.direction == Direction::Export {
                        "Export"
                    } else {
                        "Import"
                    },
                    ident::type_name(&InterfaceGenerator::func_ident(&self.func.name)),
                    kind.type_name()
                );
                self.push_line(&format!("{stem}DropReadable({});", operands[0]));
            }

            // Explicit AsyncTask implementations call the generated taskReturn
            // helper themselves when their state machine reaches completion.
            Instruction::AsyncTaskReturn { .. } => {}

            Instruction::Flush { amt } => {
                for op in operands.drain(..*amt) {
                    results.push(op);
                }
            }
        }
    }

    fn return_pointer(&mut self, size: ArchitectureSize, align: Alignment) -> Self::Operand {
        let bytes = size.size_wasm32();
        let align_bytes = align.align_wasm32();
        let wg = &mut self.iface_gen.world_gen;
        if bytes > wg.import_return_area_size {
            wg.import_return_area_size = bytes;
        }
        if align_bytes > wg.import_return_area_align {
            wg.import_return_area_align = align_bytes;
        }
        "ffi.__IMPORT_RETURN_AREA".into()
    }

    fn push_block(&mut self) {
        let prev = std::mem::take(&mut self.src);
        self.block_storage.push((prev, Vec::new()));
    }

    fn finish_block(&mut self, operands: &mut Vec<Self::Operand>) {
        let current = std::mem::take(&mut self.src);
        let (prev, _) = self.block_storage.pop().expect("finish_block without push");
        self.src = prev;
        self.blocks.push((current, std::mem::take(operands)));
    }

    fn sizes(&self) -> &SizeAlign {
        &self.iface_gen.world_gen.sizes
    }

    fn is_list_canonical(&self, _resolve: &Resolve, element: &Type) -> bool {
        matches!(
            element,
            Type::U8
                | Type::S8
                | Type::U16
                | Type::S16
                | Type::U32
                | Type::S32
                | Type::U64
                | Type::S64
                | Type::F32
                | Type::F64
        )
    }
}

fn numeric_array_lower(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::U8 => Some("u8ArrayLower"),
        Type::S8 => Some("i8ArrayLower"),
        Type::U16 => Some("u16ArrayLower"),
        Type::S16 => Some("i16ArrayLower"),
        Type::U32 => Some("u32ArrayLower"),
        Type::S32 => Some("i32ArrayLower"),
        Type::U64 => Some("u64ArrayLower"),
        Type::S64 => Some("i64ArrayLower"),
        Type::F32 => Some("f32ArrayLower"),
        Type::F64 => Some("f64ArrayLower"),
        _ => None,
    }
}

fn numeric_array_lift(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::U8 => Some("u8ArrayLift"),
        Type::S8 => Some("i8ArrayLift"),
        Type::U16 => Some("u16ArrayLift"),
        Type::S16 => Some("i16ArrayLift"),
        Type::U32 => Some("u32ArrayLift"),
        Type::S32 => Some("i32ArrayLift"),
        Type::U64 => Some("u64ArrayLift"),
        Type::S64 => Some("i64ArrayLift"),
        Type::F32 => Some("f32ArrayLift"),
        Type::F64 => Some("f64ArrayLift"),
        _ => None,
    }
}

fn bitcast_expr(cast: &Bitcast, op: &str) -> String {
    match cast {
        Bitcast::None => op.to_string(),
        Bitcast::F32ToI32 => format!("reinterpret<i32>({op})"),
        Bitcast::I32ToF32 => format!("reinterpret<f32>({op})"),
        Bitcast::F64ToI64 => format!("reinterpret<i64>({op})"),
        Bitcast::I64ToF64 => format!("reinterpret<f64>({op})"),
        Bitcast::I32ToI64 => format!("(<i64>{op})"),
        Bitcast::I64ToI32 => format!("(<i32>{op})"),
        Bitcast::F32ToI64 => format!("(<i64>reinterpret<i32>({op}))"),
        Bitcast::I64ToF32 => format!("reinterpret<f32>(<i32>{op})"),
        Bitcast::P64ToI64 | Bitcast::I64ToP64 => format!("(<i64>{op})"),
        Bitcast::P64ToP => format!("(<usize>{op})"),
        Bitcast::PToP64 => format!("(<i64>{op})"),
        Bitcast::I32ToP => format!("(<usize>{op})"),
        Bitcast::PToI32 => format!("(<i32>{op})"),
        Bitcast::PToL => format!("(<usize>{op})"),
        Bitcast::LToP => format!("(<usize>{op})"),
        Bitcast::I32ToL => format!("(<usize>{op})"),
        Bitcast::LToI32 => format!("(<i32>{op})"),
        Bitcast::I64ToL => format!("(<usize>{op})"),
        Bitcast::LToI64 => format!("(<i64>{op})"),
        Bitcast::Sequence(seq) => {
            let inner = bitcast_expr(&seq[0], op);
            bitcast_expr(&seq[1], &inner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wit_bindgen_core::wit_parser::{FunctionKind, Param, Stability, TypeDef};

    fn generate(wit: &str, world: &str, opts: Opts) -> Files {
        let mut resolve = Resolve::default();
        let pkg = resolve.push_str("test.wit", wit).unwrap();
        let world = resolve.select_world(&[pkg], Some(world)).unwrap();
        let mut files = Files::default();
        let mut generator = AssemblyScript {
            opts,
            ..AssemblyScript::default()
        };
        generator.generate(&mut resolve, world, &mut files).unwrap();
        files
    }

    fn future_type(resolve: &mut Resolve, payload: Option<Type>) -> TypeId {
        resolve.types.alloc(TypeDef {
            name: None,
            kind: TypeDefKind::Future(payload),
            owner: TypeOwner::None,
            docs: Docs::default(),
            stability: Stability::Unknown,
            span: Default::default(),
            external_id: None,
        })
    }

    fn test_function(params: Vec<(&str, Type)>) -> Function {
        Function {
            name: "f".into(),
            kind: FunctionKind::Freestanding,
            params: params
                .into_iter()
                .map(|(name, ty)| Param {
                    name: name.to_string(),
                    ty,
                    span: Default::default(),
                })
                .collect(),
            result: None,
            docs: Docs::default(),
            stability: Stability::Unknown,
            span: Default::default(),
            external_id: None,
        }
    }

    #[test]
    fn only_read_write_are_async_lowered() {
        assert!(EndpointIntrinsic::Read.async_lowered());
        assert!(EndpointIntrinsic::Write.async_lowered());
        assert!(!EndpointIntrinsic::New.async_lowered());
        assert!(!EndpointIntrinsic::CancelRead.async_lowered());
        assert!(!EndpointIntrinsic::CancelWrite.async_lowered());
        assert!(!EndpointIntrinsic::DropReadable.async_lowered());
        assert!(!EndpointIntrinsic::DropWritable.async_lowered());
    }

    #[test]
    fn cancel_intrinsics_omit_async_lower_in_canonical_name() {
        let mut resolve = Resolve::default();
        let future = future_type(&mut resolve, Some(Type::U32));
        let func = test_function(vec![("h", Type::Id(future))]);
        let field = |intrinsic: FutureIntrinsic, async_: bool| {
            resolve
                .wasm_import_name(
                    ManglingAndAbi::Legacy(LiftLowerAbi::Sync),
                    WasmImport::FutureIntrinsic {
                        interface: None,
                        func: &func,
                        ty: Some(future),
                        intrinsic,
                        exported: false,
                        async_,
                    },
                )
                .1
        };

        assert!(field(FutureIntrinsic::Read, true).contains("async-lower"));
        assert!(field(FutureIntrinsic::Write, true).contains("async-lower"));
        assert!(!field(FutureIntrinsic::CancelRead, false).contains("async-lower"));
        assert!(!field(FutureIntrinsic::CancelWrite, false).contains("async-lower"));
    }

    fn file<'a>(files: &'a Files, name: &str) -> &'a str {
        files
            .iter()
            .find(|(path, _)| *path == name)
            .map(|(_, contents)| std::str::from_utf8(contents).unwrap())
            .unwrap_or_else(|| panic!("generated `{name}`; got {:?}", paths(files)))
    }

    fn paths(files: &Files) -> Vec<&str> {
        files.iter().map(|(path, _)| path).collect()
    }

    const SPLIT_WIT: &str = r#"
        package test:bindings;

        interface api {
            resource item {
                constructor();
            }
            run: async func() -> u32;
            plain: func(x: u32) -> u32;
        }

        world test-world {
            export api;
        }
    "#;

    /// Fixtures used to be copied over `exports/<iface>.ts`, replacing every
    /// wasm-export wrapper, task base, and callback with a hand-written replica
    /// — so no generated export glue was ever executed. The user's half now
    /// lives in `stubs/<iface>.ts` and the glue imports it.
    #[test]
    fn export_glue_and_user_stub_live_in_separate_files() {
        let files = generate(SPLIT_WIT, "test-world", Opts::default());

        let glue = file(&files, "exports/test$bindings$api.ts");
        let stub = file(&files, "stubs/test$bindings$api.ts");

        // The glue owns everything the canonical ABI requires...
        assert!(glue.contains("export function __exp_"));
        assert!(glue.contains("export function __callback_"));
        assert!(glue.contains("export function __finish___exp_"));
        assert!(glue.contains("export class RunTask"));
        assert!(
            glue.contains("import { Item, item, plain, run } from \"../stubs/test$bindings$api\";")
        );
        // ...and nothing the user is meant to edit.
        assert!(!glue.contains("// TODO: implement"));
        assert!(!glue.contains("/* user fields */"));

        // The stub owns only the user's half, and reaches the generated task
        // base through the glue namespace.
        assert!(stub.contains("export function run(): e_test$bindings$api.RunTask"));
        assert!(stub.contains("return new e_test$bindings$api.RunTask();"));
        assert!(stub.contains("export class Item"));
        assert!(stub.contains("// TODO: implement"));
        assert!(!stub.contains("__exp_"));
        assert!(!stub.contains("__callback_"));

        // Exported resource classes are re-exported so sibling files can still
        // refer to them as `e_<basename>.<Type>`.
        assert!(glue.contains("export { Item };"));
    }

    /// World-level exports follow the same split, with `world.ts` as the glue.
    #[test]
    fn world_level_exports_split_into_a_stub_file() {
        let files = generate(
            r#"
                package test:bindings;

                world test-world {
                    export run: async func();
                }
            "#,
            "test-world",
            Opts::default(),
        );

        let glue = file(&files, "world.ts");
        let stub = file(&files, "stubs/world.ts");
        assert!(glue.contains("import { run } from \"./stubs/world\";"));
        assert!(glue.contains("export class RunTask"));
        assert!(stub.contains("export function run(): world.RunTask"));
        assert!(!stub.contains("__exp_"));
    }

    /// `--ignore-stub` must preserve the user's file *only*. The generated glue
    /// has to keep regenerating, or an ABI change would silently not apply.
    #[test]
    fn ignore_stub_skips_the_user_file_but_regenerates_the_glue() {
        let files = generate(
            SPLIT_WIT,
            "test-world",
            Opts {
                ignore_stub: true,
                ..Opts::default()
            },
        );

        assert!(
            !paths(&files).iter().any(|p| p.starts_with("stubs/")),
            "no stub file may be written under --ignore-stub; got {:?}",
            paths(&files)
        );
        assert!(file(&files, "exports/test$bindings$api.ts").contains("export function __exp_"));
    }

    /// The backend had no post-return at all, so every synchronous export
    /// returning a string or list leaked the buffers it handed back.
    #[test]
    fn sync_exports_returning_lists_emit_a_post_return() {
        let files = generate(
            r#"
                package test:bindings;

                world test-world {
                    export greet: func() -> string;
                    export count: func() -> u32;
                }
            "#,
            "test-world",
            Opts::default(),
        );

        let glue = file(&files, "world.ts");
        assert!(glue.contains("// wasm export: cabi_post_greet"));
        assert!(glue.contains(
            "export function __post_return_18446744073709551615_greet(a0: usize): void {"
        ));
        // The returned string's buffer is freed from the return area.
        assert!(glue.contains(
            "ffi.cabi_realloc(load<usize>(a0 + 0), <usize>(load<usize>(a0 + 4) << 1), 2, 0);"
        ));
        // Scalar results own nothing, so no post-return is emitted for them.
        assert!(!glue.contains("cabi_post_count"));

        let renames = file(&files, "wit_bindgen_exports.json");
        assert!(renames.contains("\"cabi_post_greet\""));
    }

    /// A synchronous export owns the buffers and handles the caller lowered
    /// into its memory. Lifting copies, so without an explicit release every
    /// string, list, error-context, and owned handle parameter leaked.
    #[test]
    fn sync_export_parameters_are_released_after_the_user_call() {
        let files = generate(
            r#"
                package test:bindings;

                world test-world {
                    export take: func(a: string, b: list<u32>);
                }
            "#,
            "test-world",
            Opts::default(),
        );

        let glue = file(&files, "world.ts");
        let call = glue.find("take(").expect("user call");
        let free_string = glue
            .find("ffi.cabi_realloc(a0, <usize>(a1 << 1), 2, 0);")
            .expect("string parameter buffer is freed");
        let free_list = glue
            .find("ffi.cabi_realloc(d0, <usize>(d1 * 4), 4, 0);")
            .expect("list parameter buffer is freed");
        assert!(
            call < free_string && call < free_list,
            "cleanup must follow the user call"
        );
    }

    /// A synchronous import gets `Realloc::None` because the callee only
    /// borrows. Numeric lists used to be copied into a `cabi_realloc` buffer
    /// that nothing freed, and non-canonical lists leaked their whole lowered
    /// buffer — one leak per list argument per call.
    #[test]
    fn sync_import_list_arguments_do_not_leak() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    nums: func(a: list<u32>);
                    strs: func(a: list<string>);
                    owned: async func(a: list<u32>);
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let imports = file(&files, "imports/test$bindings$api.ts");

        // Numeric list: borrowed in place rather than copied.
        let nums = imports
            .split("export function nums(")
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .expect("sync numeric-list import");
        assert!(nums.contains("changetype<usize>(v0.dataStart)"));
        assert!(!nums.contains("u32ArrayLower"));

        // Non-canonical list: the lowered buffer and each element's string
        // buffer are released, and only after the call.
        let call = imports.find("__ext_strs(").expect("import call");
        let free_elems = imports
            .find("ffi.cabi_realloc(load<usize>(v7 + 0), <usize>(load<usize>(v7 + 4) << 1), 2, 0);")
            .expect("element string buffers freed");
        assert!(call < free_elems);

        // The async lowering hands ownership to the callee, so it must still
        // copy and must not free at the call site.
        assert!(imports.contains("ffi.u32ArrayLower("));
    }

    /// Two hard limits of the backend, pinned so that raising them (or turning
    /// them into a graceful error) is a deliberate change rather than a
    /// surprise in the field. `ffi.ts` defines `Tuple1`..`Tuple16`, and
    /// AssemblyScript has no integer wider than 64 bits to hold a flags value.
    #[test]
    #[should_panic(expected = "arity 17 > 16")]
    fn tuples_wider_than_sixteen_are_rejected() {
        let types = (0..17)
            .map(|i| format!("t{i}: u8"))
            .collect::<Vec<_>>()
            .join(", ");
        generate(
            &format!(
                r#"
                package test:bindings;

                world test-world {{
                    export wide: func() -> tuple<{}>;
                }}
                "#,
                vec!["u8"; 17].join(", ")
            )
            .replace("UNUSED", &types),
            "test-world",
            Opts::default(),
        );
    }

    #[test]
    #[should_panic(expected = "flag count > 64")]
    fn flags_wider_than_sixty_four_are_rejected() {
        let members = (0..65)
            .map(|i| format!("flag{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        generate(
            &format!(
                r#"
                package test:bindings;

                interface api {{
                    flags wide {{ {members} }}
                    take: func(a: wide);
                }}

                world test-world {{
                    export api;
                }}
                "#
            ),
            "test-world",
            Opts::default(),
        );
    }

    #[test]
    fn async_import_drop_helpers_follow_duplicate_endpoint_occurrences() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    type f = future<u32>;
                    run: func(a: f, b: f) -> f;
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts {
                async_: AsyncFilterSet::all(true),
                ..Opts::default()
            },
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .find(|contents| contents.contains("class RunSubtask"))
            .expect("generated async import subtask");

        let param0 = generated
            .find("rawImportRunFuture0DropReadable((<i32>this.a0));")
            .expect("first parameter uses endpoint helper 0");
        let param1 = generated
            .find("rawImportRunFuture1DropReadable((<i32>this.a1));")
            .expect("second parameter uses endpoint helper 1");
        let result = generated
            .find("rawImportRunFuture2DropReadable((<i32>load<i32>(this.result + 0)))")
            .expect("result uses endpoint helper 2");
        assert!(param0 < param1 && param1 < result);
    }

    #[test]
    fn async_import_cancel_is_exactly_once() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    run: async func();
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .find(|contents| contents.contains("class RunSubtask"))
            .expect("generated async import subtask");

        assert!(generated.contains("private cancellationRequested: bool = false;"));
        assert!(generated.contains(
            "if (!this.started || this.finished || this.cancellationRequested || this.handle == 0) unreachable();"
        ));
        assert!(generated.contains("this.cancellationRequested = true;"));
    }

    #[test]
    fn error_context_uses_typed_wrapper_and_drop() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    round-trip: func(context: error-context) -> error-context;
                }

                world test-world {
                    export api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(generated.contains("async_.ErrorContext"));
        assert!(generated.contains("new async_.ErrorContext"));
        assert!(generated.contains(".handle"));
        // The export receives an owned error-context. Lifting only wraps the
        // handle, so the wrapper has to release it after the user call — this
        // assertion is the point of the test and used to be missing.
        assert!(
            generated.contains("new async_.ErrorContext(<i32>a0).drop();"),
            "an owned error-context parameter must be dropped"
        );
    }

    /// An exiting async export must perform exactly one of `task.return` or
    /// `task.cancel`. Before this was fixed the scheduler called `task.cancel`
    /// eagerly on EVENT_CANCEL and then resumed the task anyway, so a task that
    /// completed during cancellation issued both and trapped.
    #[test]
    fn async_export_exit_issues_exactly_one_of_return_or_cancel() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    run: async func();
                }

                world test-world {
                    export api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .find(|contents| contents.contains("function __finish___exp_0_run"))
            .expect("generated async export finish helper");

        assert!(generated.contains("} else if (async_.Scheduler.wasCancelled()) {"));
        assert!(generated.contains("async_.taskCancel();"));
        // Exiting with neither is a canonical-ABI violation; trap in the guest
        // rather than letting the host trap without context.
        assert!(generated.contains("} else {\n    unreachable();"));
        // context-0 must be cleared before the task allocation is freed.
        let complete = generated.find("Scheduler.complete(task)").unwrap();
        let release = generated.find("Scheduler.release(task)").unwrap();
        assert!(complete < release, "complete must precede release");
    }

    /// `abi::deallocate` lifts an owned handle before emitting `DropHandle`, so
    /// the operand is already the wrapper instance. Re-wrapping it produced
    /// `new Item(new Item(h)).drop()`, which does not typecheck.
    #[test]
    fn owned_handle_cleanup_does_not_rewrap_lifted_operand() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    resource item;
                    consume: async func(value: own<item>);
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !generated.contains("new Item(new Item("),
            "owned handle cleanup must not double-wrap the lifted operand"
        );
    }

    /// The variant deallocation instruction used to discard every payload block
    /// without emitting the tag switch, so the active payload — and any owned
    /// handle inside it — was never released.
    #[test]
    fn variant_deallocation_emits_tag_switch_over_payload_blocks() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    handle: async func() -> option<string>;
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            generated.contains("switch (<i32>"),
            "variant deallocation must switch on the tag"
        );
    }

    /// List deallocation used to drop the per-element block and free the buffer
    /// with the element *count* as its byte size.
    #[test]
    fn list_deallocation_frees_elements_and_uses_byte_size() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    handle: async func() -> list<string>;
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        // Each element's string buffer must be freed inside a loop, and the
        // outer free must scale the length by the element size (8 bytes for a
        // string's ptr/len pair) rather than passing the element count.
        assert!(
            generated.contains("* 8), 4, 0);"),
            "outer list free must use len * elem_size and the element alignment"
        );
    }

    /// `exportStart` suppresses the wasm `(start)` section and exports `_start`
    /// for an embedder to call. Nothing calls it in a component, so the
    /// AssemblyScript runtime stayed uninitialized and the first managed
    /// allocation aborted — which is why only all-scalar fixtures ever passed.
    #[test]
    fn asconfig_emits_a_start_section_so_the_runtime_is_initialized() {
        let files = generate(
            r#"
                package test:bindings;

                world test-world {
                    export run: func();
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let asconfig = files
            .iter()
            .find(|(name, _)| *name == "asconfig.json")
            .map(|(_, contents)| String::from_utf8(contents.to_vec()).unwrap())
            .expect("generated asconfig.json");

        assert!(
            !asconfig.contains("exportStart"),
            "asconfig must not set exportStart, got: {asconfig}"
        );
    }

    #[test]
    fn stream_payload_helpers_keep_canonical_payload_pointer() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    type bytes = stream<u8>;
                    consume: func(value: bytes);
                }

                world test-world {
                    export api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(generated.contains("rawExportConsumeStream0Read"));
        assert!(generated.contains("payload: usize"));
    }

    #[test]
    fn async_import_cleanup_drops_imported_owned_resources() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    resource item;
                    run: async func(value: own<item>);
                }

                world test-world {
                    import api;
                }
            "#,
            "test-world",
            Opts {
                async_: AsyncFilterSet::all(true),
                ..Opts::default()
            },
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        // A bare `drop();` substring also matches the double-wrapped form that
        // used to be emitted, so assert on the exact shape and pin the absence
        // of the re-wrap.
        assert!(
            generated.contains("new Item("),
            "expected the imported resource wrapper class, got: {generated}"
        );
        assert!(
            !generated.contains("new Item(new Item("),
            "cleanup must drop the already-lifted wrapper, got: {generated}"
        );
    }

    /// Owned-handle cleanup only runs on the async *import* path, so a resource
    /// the guest exports reaches it by travelling out through an imported async
    /// function.
    ///
    /// KNOWN GAP: in this shape the generator resolves the `use`d resource to an
    /// *imported* wrapper class and lowers it via a raw `.handle`, instead of
    /// routing it through the exported instance table (`__Item_take` /
    /// `__Item_drop_instance`). The assertions below pin the cleanup that is
    /// actually emitted today; see `exported_resource_through_async_import_is_not_yet_routed_through_the_instance_table`.
    #[test]
    fn exported_owned_resource_cleanup_uses_instance_table() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    resource item;
                }

                interface consumer {
                    use api.{item};
                    run: async func(value: own<item>);
                }

                world test-world {
                    export api;
                    import consumer;
                }
            "#,
            "test-world",
            Opts {
                async_: AsyncFilterSet::all(true),
                ..Opts::default()
            },
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        // The exported instance table is emitted for the resource.
        assert!(generated.contains("Map<Item, i32>"));
        assert!(generated.contains("export function __Item_drop_instance("));
        // Owned params are released exactly once, only on the cancelled path,
        // and the operand is the already-lifted wrapper (not re-wrapped).
        assert!(generated.contains("private releaseParams(own: bool): void {"));
        assert!(generated.contains(".drop();"));
        assert!(!generated.contains("new i_test$bindings$api.Item(new "));
    }

    /// A world that exports an interface *and* imports something using that
    /// interface's resource ends up with two distinct resource types: the
    /// imported view and the exported one. The import must therefore lower a
    /// raw handle on the imported wrapper rather than acquire one from the
    /// exported instance table — `wit-bindgen-rust` splits the same world into
    /// `test::bindings::api::Item` and `exports::test::bindings::api::Item` for
    /// exactly this reason.
    ///
    /// Previously read as a known gap ("not yet routed through the instance
    /// table"); it is the correct lowering.
    #[test]
    fn imported_and_exported_views_of_a_resource_are_distinct_types() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    resource item;
                }

                interface consumer {
                    use api.{item};
                    run: async func(value: own<item>);
                }

                world test-world {
                    export api;
                    import consumer;
                }
            "#,
            "test-world",
            Opts {
                async_: AsyncFilterSet::all(true),
                ..Opts::default()
            },
        );
        let generated = files
            .iter()
            .filter_map(|(_, contents)| std::str::from_utf8(contents).ok())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            generated.contains("const subtask = new RunSubtask(value.handle);"),
            "the imported view lowers its own handle, got: {generated}"
        );
        assert!(
            generated.contains("export function run(value: i_test$bindings$api.Item)"),
            "the import takes the imported wrapper, not the exported class"
        );
    }

    /// A synchronous export releases its owned parameters right after the user
    /// call, but an async export's task outlives the wrapper, so the release is
    /// driven from the finish helper — on both the returned and the cancelled
    /// path. Previously nothing released them and the instance table leaked an
    /// entry per call.
    #[test]
    fn async_export_releases_owned_parameters_when_its_task_exits() {
        let files = generate(
            r#"
                package test:bindings;

                interface api {
                    resource item;
                    consume: async func(value: own<item>, note: string);
                }

                world test-world {
                    export api;
                }
            "#,
            "test-world",
            Opts::default(),
        );
        let glue = file(&files, "exports/test$bindings$api.ts");

        // The raw arguments are persisted on the task...
        assert!(glue.contains("__arg0: i32 = 0;"));
        assert!(glue.contains("__arg1: usize = 0;"));
        assert!(glue.contains("v0.__arg0 = a0;"));
        // ...and read back by the finish helper, before the task is freed.
        let drop_handle = glue
            .find("__Item_drop_instance(__Item_get(load<i32>(task + offsetof<ConsumeTask>(\"__arg0\"))));")
            .expect("owned handle released from the finish helper");
        let free_string = glue
            .find("ffi.cabi_realloc(load<usize>(task + offsetof<ConsumeTask>(\"__arg1\"))")
            .expect("string parameter buffer released from the finish helper");
        let release = glue
            .find("async_.Scheduler.release(task);")
            .expect("task freed");
        assert!(drop_handle < release && free_string < release);
    }

    #[test]
    fn scheduler_rejects_reentrant_tasks() {
        assert!(r#async::ASYNC_TS.contains(
            "if (contextGet() != 0) unreachable();\n    const ptr = changetype<usize>(task);"
        ));
    }
}

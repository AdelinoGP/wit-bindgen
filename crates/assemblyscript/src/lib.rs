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
//! # Async
//!
//! Async callback bindings use an explicit stackless `AsyncTask` API because
//! AssemblyScript has no native async/await. `future<T>`, `stream<T>`, and
//! `error-context` remain opaque handles; payload read/write operations are not
//! generated yet.

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::mem;
use wit_bindgen_core::abi::{self, AbiVariant, Bindgen, Bitcast, Instruction, LiftLower, WasmType};
use wit_bindgen_core::wit_parser::{
    Alignment, ArchitectureSize, Docs, Enum, Flags, FlagsRepr, Function, Handle, InterfaceId,
    LiftLowerAbi, Mangling, ManglingAndAbi, Record, Resolve, Result_, SizeAlign, Tuple, Type,
    TypeDefKind, TypeId, TypeOwner, Variant, WasmExport, WasmExportKind, WorldId, WorldKey,
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
            let body = render_interface_file(frag, "imports");
            files.push(&path, body.as_bytes());
        }

        // Emit exports/<basename>.ts files (stubs the user edits).
        // Default policy is to overwrite. Under --ignore-stub we skip.
        let export_interfaces = mem::take(&mut self.export_interfaces);
        if !self.opts.ignore_stub {
            for (basename, frag) in &export_interfaces {
                let path = format!("exports/{basename}.ts");
                let body = render_interface_file(frag, "exports");
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
            let body = render_interface_file(&combined, "world");
            files.push("world.ts", body.as_bytes());
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
            "{{\n  \"targets\": {{\n    \"release\": {{\n      \"outFile\": \"core.wasm\",\n      \"runtime\": \"{runtime}\",\n      \"exportRuntime\": true,\n      \"exportStart\": \"_start\",\n      \"optimizeLevel\": 3,\n      \"shrinkLevel\": 0,\n      \"converge\": false,\n      \"noAssert\": false,\n      \"use\": [\"abort=ffi/abort\"]\n    }},\n    \"debug\": {{\n      \"outFile\": \"core.wasm\",\n      \"runtime\": \"{runtime}\",\n      \"exportRuntime\": true,\n      \"exportStart\": \"_start\",\n      \"debug\": true,\n      \"use\": [\"abort=ffi/abort\"]\n    }}\n  }}\n}}\n",
            runtime = self.opts.runtime.as_asc_name()
        )
    }
}

fn render_interface_file(frag: &InterfaceFragment, kind: &str) -> String {
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
    for basename in &frag.imports_imports {
        writeln!(
            s,
            "import * as i_{basename} from \"{prefix}imports/{basename}\";"
        )
        .unwrap();
    }
    for basename in &frag.imports_exports {
        writeln!(
            s,
            "import * as e_{basename} from \"{prefix}exports/{basename}\";"
        )
        .unwrap();
    }
    writeln!(s).unwrap();
    s.push_str(&frag.src);
    s
}

impl InterfaceFragment {
    fn concat(&mut self, other: InterfaceFragment) {
        self.src.push_str(&other.src);
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
            imports_imports: BTreeSet::new(),
            imports_exports: BTreeSet::new(),
            needs_async: false,
        }
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
            Type::ErrorContext => "i32".into(),
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
            _ => local,
        }
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
        let raw_extern_name = if is_async {
            format!("[async-lower]{}", func.name)
        } else {
            func.name.clone()
        };
        let wasm_module = self.wasm_import_module(func);
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

        // Friendly wrapper: lifts args from AS values into wasm types, calls
        // the import, lifts results back.
        let wrapper = if is_async {
            sig.async_import_signature(&wrapper_name)
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
            let result_ptr = if let Some(result) = &func.result {
                let layout = bindgen.iface_gen.world_gen.sizes.record([result]);
                bindgen.push_line(&format!(
                    "const __result = ffi.cabi_realloc(0, 0, {}, {});",
                    layout.align.align_wasm32(),
                    layout.size.size_wasm32()
                ));
                wasm_params.push("__result".into());
                "__result"
            } else {
                "0"
            };
            bindgen.push_line(&format!(
                "const __status = {mangled_extern}({});",
                wasm_params.join(", ")
            ));
            bindgen.push_line(&format!(
                "return new async_.AsyncSubtask(__status, <usize>{result_ptr});"
            ));
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
        let variant = if is_async {
            AbiVariant::GuestExportAsync
        } else {
            AbiVariant::GuestExport
        };
        let sig = self.func_signature(func, variant);

        self.docs(&func.docs);
        if is_async {
            writeln!(
                self.src,
                "/// Return an explicit state machine; the host resumes it through `resume`."
            )
            .unwrap();
        }
        let user_sig = if is_async {
            sig.async_export_signature(&user_ident)
        } else {
            sig.user_signature(&user_ident)
        };
        writeln!(self.src, "export function {user_sig} {{").unwrap();
        if is_async {
            writeln!(self.src, "  return new async_.UnimplementedTask();").unwrap();
        } else {
            writeln!(self.src, "  // TODO: implement").unwrap();
            if let Some(ret_ty) = &sig.return_type {
                writeln!(self.src, "  return {};", default_value_for(ret_ty)).unwrap();
            }
        }
        writeln!(self.src, "}}").unwrap();
        writeln!(self.src).unwrap();

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
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                variant,
                LiftLower::LiftArgsLowerResults,
                user_ident.clone(),
            );
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
            let user_call = user_ident.clone();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestExport,
                LiftLower::LiftArgsLowerResults,
                user_call,
            );
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
            self.emit_async_export_support(func, &func_local);
        }
    }

    fn emit_async_export_support(&mut self, func: &Function, func_local: &str) {
        let safe = sanitize_extern_local(&format!(
            "{}_{}",
            self.interface.map(|id| id.index()).unwrap_or(usize::MAX),
            func_local
        ));
        let task_return_extern = format!("__task_return_{safe}");
        let task_return_helper = format!("taskReturn{}", ident::type_name(func_local));
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

        if let Some(result) = &func.result {
            let result_ty = self.type_ref(result);
            writeln!(
                self.src,
                "export function {task_return_helper}(result: {result_ty}): void {{"
            )
            .unwrap();
            let mut bindgen = FunctionBindgen::new(
                self,
                func,
                AbiVariant::GuestExportAsync,
                LiftLower::LowerArgsLiftResults,
                task_return_extern.clone(),
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
            bindgen.push_line(&format!("{task_return_extern}({});", args.join(", ")));
            bindgen.push_line("async_.Scheduler.complete();");
            for line in bindgen.into_body().lines() {
                writeln!(self.src, "  {line}").unwrap();
            }
            writeln!(self.src, "}}").unwrap();
        } else {
            writeln!(
                self.src,
                "export function {task_return_helper}(): void {{ {task_return_extern}(); async_.Scheduler.complete(); }}"
            )
            .unwrap();
        }
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
            "  return async_.Scheduler.resume(event, waitable, code);"
        )
        .unwrap();
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

    fn wasm_import_module(&self, _func: &Function) -> String {
        match &self.interface_key {
            Some(key) => self.resolve.name_world_key(key),
            None => "$root".to_string(),
        }
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

    fn async_import_signature(&self, name: &str) -> String {
        let ps = self
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{name}({ps}): async_.AsyncSubtask")
    }

    fn async_export_signature(&self, name: &str) -> String {
        let ps = self
            .params
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{name}({ps}): async_.AsyncTask")
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
                writeln!(
                    self.src,
                    "// Exported resource `{name}` — implement this class and"
                )
                .unwrap();
                writeln!(
                    self.src,
                    "// add an optional `__onDrop()` method for cleanup."
                )
                .unwrap();
                writeln!(self.src, "export class {cls} {{").unwrap();
                writeln!(self.src, "  constructor() {{ /* user fields */ }}").unwrap();
                writeln!(self.src, "  __onDrop(): void {{}}").unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src).unwrap();

                // The handle table + new/drop wrappers, inlined in the same file
                // so they have direct access to the class:
                let safe = sanitize_extern_local(&cls);
                writeln!(
                    self.src,
                    "const __{safe}_table: Map<i32, {cls}> = new Map<i32, {cls}>();"
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
                    "  const h = __{safe}_next++; __{safe}_table.set(h, inst); return h;"
                )
                .unwrap();
                writeln!(self.src, "}}").unwrap();
                writeln!(self.src, "export function __{safe}_get(h: i32): {cls} {{ return __{safe}_table.get(h)!; }}").unwrap();
                writeln!(self.src, "export function __{safe}_drop(h: i32): void {{").unwrap();
                writeln!(self.src, "  const inst = __{safe}_table.get(h);").unwrap();
                writeln!(
                    self.src,
                    "  if (inst !== null) {{ inst.__onDrop(); __{safe}_table.delete(h); }}"
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
            src: String::new(),
            blocks: Vec::new(),
            block_storage: Vec::new(),
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_local;
        self.next_local += 1;
        format!("v{n}")
    }

    fn into_body(self) -> String {
        self.src
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
                    self.push_line(&format!("const {ptr} = ffi.{helper}({arr_ref});"));
                    self.push_line(&format!("const {len} = <i32>{arr_ref}.length;"));
                    let _ = realloc;
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
                let _ = realloc;
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

            // Futures / streams / error-context — opaque i32 pass-through.
            Instruction::FutureLower { .. }
            | Instruction::StreamLower { .. }
            | Instruction::ErrorContextLower => {
                results.push(format!("(<i32>{})", operands[0]));
            }
            Instruction::FutureLift { .. }
            | Instruction::StreamLift { .. }
            | Instruction::ErrorContextLift => {
                results.push(format!("(<i32>{})", operands[0]));
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
            }

            Instruction::CallInterface { func, async_ } => {
                let args = operands.drain(..).collect::<Vec<_>>().join(", ");
                if *async_ {
                    let task = self.fresh();
                    self.push_line(&format!("const {task} = {}({args});", self.call_target));
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
            Instruction::GuestDeallocateList { .. } => {
                // Skip the per-element deallocation loop in v1; just free the
                // outer buffer with size = len * elem_size (caller knows neither
                // size nor align here without the block — use 1, 1 as a
                // best-effort).
                self.push_line(&format!(
                    "ffi.cabi_realloc({}, <usize>{}, 1, 0);",
                    operands[0], operands[1]
                ));
                // The block was already popped at push_block time; nothing else
                // to do here.
                self.blocks.pop();
            }
            Instruction::GuestDeallocateMap { .. } => {
                self.blocks.pop();
                self.push_line(&format!(
                    "ffi.cabi_realloc({}, <usize>{}, 1, 0);",
                    operands[0], operands[1]
                ));
            }
            Instruction::GuestDeallocateVariant { blocks } => {
                for _ in 0..*blocks {
                    self.blocks.pop();
                }
                let _ = operands;
            }

            Instruction::DropHandle { .. } => {
                let _ = operands;
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

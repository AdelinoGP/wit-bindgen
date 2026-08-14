use crate::{Compile, LanguageMethods, Runner, Verify};
use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

pub struct AssemblyScript;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LangConfig {
    /// Path under `bindings_dir` to write the user's test/runner file to.
    ///
    /// E.g. `path = "stubs/foo$foo$records.ts"` replaces the generated
    /// user-implementation stub at that path.
    ///
    /// A world exporting several interfaces needs one file per interface. Split
    /// the fixture with `// @@file: <path>` marker lines: everything before the
    /// first marker goes to `path`, and each following section goes to the path
    /// its marker names.
    #[serde(default)]
    path: String,
}

/// Marker introducing an additional output file inside a fixture.
const FILE_MARKER: &str = "// @@file:";

/// Split a fixture into `(path, contents)` pairs. The first section takes
/// `default_path`; later sections take the path from their marker line.
fn split_fixture(source: &str, default_path: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut path = default_path.to_string();
    let mut body = String::new();
    for line in source.lines() {
        if let Some(next) = line.trim().strip_prefix(FILE_MARKER) {
            files.push((std::mem::take(&mut path), std::mem::take(&mut body)));
            path = next.trim().to_string();
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    files.push((path, body));
    files
}

impl LanguageMethods for AssemblyScript {
    fn display(&self) -> &str {
        "assemblyscript"
    }

    fn comment_prefix_for_test_config(&self) -> Option<&str> {
        Some("//@")
    }

    fn should_fail_verify(
        &self,
        _runner: &Runner,
        _name: &str,
        _config: &crate::config::WitConfig,
        _args: &[String],
    ) -> bool {
        // Verification only type-checks the generated source. Unsupported
        // async functions and opaque async types are still valid AS syntax.
        false
    }

    fn prepare(&self, runner: &mut Runner) -> Result<()> {
        println!("Testing if the AssemblyScript toolchain (`asc`) is installed...");
        if runner
            .run_command(Command::new(asc_binary()).arg("--version"))
            .is_err()
        {
            bail!(
                "AssemblyScript compiler `asc` not found on PATH. \
                 Install it with `npm i -g assemblyscript`."
            );
        }
        Ok(())
    }

    fn default_bindgen_args(&self) -> &[&str] {
        // The runtime variant flows from $WIT_BINDGEN_AS_RUNTIME, applied below
        // because clap-arg-defaults can't read env vars statically. This is the
        // codegen-time default (no extra flags).
        &[]
    }

    fn codegen_test_variants(&self) -> &[(&str, &[&str])] {
        &[("async", &["--async", "all"])]
    }

    fn compile(&self, runner: &Runner, compile: &Compile<'_>) -> Result<()> {
        // Pull the user's test/runner file into the expected slot under the
        // generated `exports/` tree.
        let cfg = compile.component.deserialize_lang_config::<LangConfig>()?;
        if !cfg.path.is_empty() {
            let source = std::fs::read_to_string(&compile.component.path).with_context(|| {
                format!("unable to read `{}`", compile.component.path.display())
            })?;
            for (path, body) in split_fixture(&source, &cfg.path) {
                if path.is_empty() {
                    bail!("`{FILE_MARKER}` marker without a path");
                }
                let dest = compile.bindings_dir.join(&path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("unable to create `{}`", parent.display()))?;
                }
                std::fs::write(&dest, body)
                    .with_context(|| format!("failed to write `{}`", dest.display()))?;
            }
        }

        // Run asc, driven by the generated asconfig.json. `WIT_BINDGEN_AS_RUNTIME`
        // (env var) selects `incremental` vs `minimal`; default `incremental`.
        let core_wasm = compile.bindings_dir.join("core.wasm");
        let runtime =
            std::env::var("WIT_BINDGEN_AS_RUNTIME").unwrap_or_else(|_| "incremental".into());
        let mut cmd = Command::new(asc_binary());
        cmd.arg("bindings.ts")
            .arg("--target")
            .arg("release")
            .arg("--outFile")
            .arg(&core_wasm)
            .arg("--runtime")
            .arg(&runtime)
            .current_dir(compile.bindings_dir);
        runner.run_command(&mut cmd)?;

        // Rewrite wasm export-section names from AS identifiers to the
        // canonical WIT names recorded in wit_bindgen_exports.json.
        let rename_json = compile.bindings_dir.join("wit_bindgen_exports.json");
        let rename_src = std::fs::read_to_string(&rename_json)
            .with_context(|| format!("unable to read `{}`", rename_json.display()))?;
        let renames: HashMap<String, String> = serde_json::from_str(&rename_src)
            .context("failed to parse wit_bindgen_exports.json")?;
        let raw_wasm = std::fs::read(&core_wasm)?;
        let renamed = rewrite_export_names(&raw_wasm, &renames)?;
        std::fs::write(&core_wasm, renamed)?;

        // Embed WIT into the module and componentize.
        let manifest_dir = compile.component.path.parent().unwrap();
        let embedded = core_wasm.with_extension("embedded.wasm");
        let mut cmd = Command::new("wasm-tools");
        cmd.arg("component")
            .arg("embed")
            .args(["--encoding", "utf16"])
            .args(["-o", embedded.to_str().unwrap()])
            .args(["-w", &compile.component.bindgen.world])
            .arg(manifest_dir)
            .arg(&core_wasm);
        runner.run_command(&mut cmd)?;

        let mut cmd = Command::new("wasm-tools");
        cmd.arg("component")
            .arg("new")
            .args(["-o", compile.output.to_str().unwrap()])
            .arg(&embedded);
        runner.run_command(&mut cmd)?;

        Ok(())
    }

    fn verify(&self, runner: &Runner, verify: &Verify<'_>) -> Result<()> {
        // Type-check the generated bindings without producing wasm.
        runner.run_command(
            Command::new(asc_binary())
                .arg("bindings.ts")
                .arg("--noEmit")
                .current_dir(verify.bindings_dir),
        )?;
        Ok(())
    }
}

/// Locate the `asc` binary. On Windows the npm-global install lands an `asc.cmd`
/// shim; bare `asc` works elsewhere.
fn asc_binary() -> &'static str {
    if cfg!(windows) { "asc.cmd" } else { "asc" }
}

/// Rewrite the wasm export section, renaming any export whose name is a key in
/// `renames` to the corresponding value. All other sections pass through
/// untouched. Used to bridge the gap between asc's identifier-only export names
/// and the canonical WIT names (e.g. `foo:foo/records#tuple-arg`).
fn rewrite_export_names(wasm: &[u8], renames: &HashMap<String, String>) -> Result<Vec<u8>> {
    use wasm_encoder::{ExportKind, ExportSection, Module, RawSection};
    use wasmparser::{ExternalKind, Parser, Payload};

    let mut module = Module::new();
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.context("failed to parse wasm")?;
        match payload {
            Payload::ExportSection(reader) => {
                let mut section = ExportSection::new();
                for export in reader {
                    let export = export.context("invalid export entry")?;
                    let name = renames
                        .get(export.name)
                        .map(String::as_str)
                        .unwrap_or(export.name);
                    let kind = match export.kind {
                        ExternalKind::Func | ExternalKind::FuncExact => ExportKind::Func,
                        ExternalKind::Table => ExportKind::Table,
                        ExternalKind::Memory => ExportKind::Memory,
                        ExternalKind::Global => ExportKind::Global,
                        ExternalKind::Tag => ExportKind::Tag,
                    };
                    section.export(name, kind, export.index);
                }
                module.section(&section);
            }
            Payload::Version { .. } | Payload::End(_) | Payload::CodeSectionEntry(_) => {
                // Version/End are bookkeeping; CodeSectionEntry is covered by
                // CodeSectionStart's raw range. Skip.
            }
            other => {
                if let Some((id, range)) = other.as_section() {
                    module.section(&RawSection {
                        id,
                        data: &wasm[range],
                    });
                }
            }
        }
    }
    Ok(module.finish())
}

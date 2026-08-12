//! Identifier mangling utilities for translating WIT names into AssemblyScript.
//!
//! WIT uses kebab-case. AssemblyScript follows TypeScript conventions:
//!   - camelCase for functions, fields, parameters, variant payload binds.
//!   - PascalCase for types, resources, enum cases, variant cases.
//!
//! TypeScript / AssemblyScript reserved words that collide with WIT identifiers
//! get a trailing underscore.

use heck::{ToLowerCamelCase, ToUpperCamelCase};

/// AssemblyScript / TypeScript reserved keywords and built-in identifiers that
/// must not be used unescaped as value-name positions (fields, parameters,
/// locals, function names).
const RESERVED: &[&str] = &[
    // ES + TS keywords
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "constructor",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "module",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "object",
    "of",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "return",
    "set",
    "static",
    "string",
    "super",
    "switch",
    "symbol",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unique",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
    // AS-specific
    "i8",
    "u8",
    "i16",
    "u16",
    "i32",
    "u32",
    "i64",
    "u64",
    "isize",
    "usize",
    "f32",
    "f64",
    "bool",
    "v128",
    "anyref",
    "externref",
    // AS globals we generate, plus a defensive list of frequently-used builtins
    "abort",
    "load",
    "store",
    "memory",
    "heap",
    "changetype",
    "unreachable",
    "global",
];

fn is_reserved(name: &str) -> bool {
    RESERVED.binary_search(&name).is_ok() || RESERVED.iter().any(|r| *r == name)
}

/// Escape a WIT value-name (function, field, parameter) to camelCase AS.
pub fn value_name(wit: &str) -> String {
    let mut out = wit.to_lower_camel_case();
    if out.is_empty() {
        return "_".to_string();
    }
    if is_reserved(&out) {
        out.push('_');
    }
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    out
}

/// Escape a WIT type-name (record, variant, resource, enum, flags) to PascalCase AS.
pub fn type_name(wit: &str) -> String {
    let mut out = wit.to_upper_camel_case();
    if out.is_empty() {
        return "_".to_string();
    }
    if is_reserved(&out) {
        out.push('_');
    }
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    out
}

/// Escape a variant or enum case to PascalCase.
pub fn case_name(wit: &str) -> String {
    type_name(wit)
}

/// Sanitize a WIT package + interface identifier into a filesystem-safe basename.
///
/// `wasi:io/streams@0.2.0` becomes `wasi$io$streams$0_2_0`. Used both as the
/// file basename under `imports/` / `exports/` and as the namespace-import alias.
pub fn iface_basename(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            ':' | '/' | '@' => out.push('$'),
            '.' | '-' | ' ' => out.push('_'),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '$' => out.push(c),
            _ => out.push('_'),
        }
    }
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_basic() {
        assert_eq!(value_name("do-thing"), "doThing");
        assert_eq!(value_name("get-status-code"), "getStatusCode");
    }

    #[test]
    fn reserved_words_get_suffix() {
        assert_eq!(value_name("class"), "class_");
        assert_eq!(value_name("function"), "function_");
        assert_eq!(value_name("string"), "string_");
    }

    #[test]
    fn type_names_are_pascal() {
        assert_eq!(type_name("my-record"), "MyRecord");
        assert_eq!(type_name("io-error"), "IoError");
    }

    #[test]
    fn iface_basename_uses_dollar_separators() {
        assert_eq!(iface_basename("wasi:io/streams"), "wasi$io$streams");
        assert_eq!(
            iface_basename("wasi:io/streams@0.2.0"),
            "wasi$io$streams$0_2_0"
        );
    }
}

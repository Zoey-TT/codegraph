//! Rust language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{ExtractedSymbol, SymbolExtractor};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

// ============================================================================
// Queries
// ============================================================================

const RUST_QUERIES: &str = r#"
;; Definitions
(function_item name: (identifier) @definition.function) @def.function
(struct_item name: (type_identifier) @definition.struct) @def.struct
(enum_item name: (type_identifier) @definition.enum) @def.enum
(trait_item name: (type_identifier) @definition.trait) @def.trait
(impl_item type: (type_identifier) @definition.impl) @def.impl
(macro_definition name: (identifier) @definition.macro) @def.macro
(const_item name: (identifier) @definition.const) @def.const
(static_item name: (identifier) @definition.static) @def.static
(type_item name: (type_identifier) @definition.type) @def.typealias
"#;

// ============================================================================
// RustProvider
// ============================================================================

pub struct RustProvider;

impl LanguageProvider for RustProvider {
    fn id(&self) -> Language {
        Language::Rust
    }

    fn extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn tree_sitter_queries(&self) -> &str {
        RUST_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::Named
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::None
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(RustSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn crate::extractors::CallExtractor>> {
        Some(Box::new(RustCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn crate::extractors::ImportExtractor>> {
        Some(Box::new(RustImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn crate::extractors::HeritageExtractor>> {
        Some(Box::new(RustHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        resolve_rust_import(target, from, all_files)
    }

    fn is_exported(&self, _name: &str, node: &Node, _source: &[u8]) -> bool {
        let mut current = *node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "visibility_modifier" {
                return true;
            }
            if [
                "function_item",
                "struct_item",
                "enum_item",
                "trait_item",
                "impl_item",
                "const_item",
                "static_item",
                "type_item",
            ]
            .contains(&parent.kind())
            {
                for i in 0..parent.child_count() {
                    if let Some(child) = parent.child(i)
                        && child.kind() == "visibility_modifier"
                    {
                        return true;
                    }
                }
                return false;
            }
            current = parent;
        }
        false
    }
}

// ============================================================================
// Symbol Extractor
// ============================================================================

struct RustSymbolExtractor;

impl SymbolExtractor for RustSymbolExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = match build_query(grammar, RUST_QUERIES) {
            Ok(q) => q,
            Err(_) => return symbols,
        };

        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, parsed.tree.root_node(), &parsed.source[..]);
        let mut captures = Vec::new();
        loop {
            let m = iter.next();
            if m.is_none() {
                break;
            }
            let m = m.unwrap();
            for capture in m.captures {
                captures.push((capture.index, capture.node));
            }
        }

        for (idx, node) in captures {
            let capture_name = query.capture_names()[idx as usize];
            let name = node.utf8_text(&parsed.source[..]).unwrap_or("").to_string();

            let (kind, def_node) = match capture_name {
                "definition.function" => (
                    NodeKind::Function,
                    parent_kind(node, &["function_item", "method_item"]).unwrap_or(node),
                ),
                "definition.struct" => (
                    NodeKind::Struct,
                    parent_kind(node, &["struct_item"]).unwrap_or(node),
                ),
                "definition.enum" => (
                    NodeKind::Enum,
                    parent_kind(node, &["enum_item"]).unwrap_or(node),
                ),
                "definition.trait" => (
                    NodeKind::Trait,
                    parent_kind(node, &["trait_item"]).unwrap_or(node),
                ),
                "definition.impl" => (
                    NodeKind::Impl,
                    parent_kind(node, &["impl_item"]).unwrap_or(node),
                ),
                "definition.macro" => (
                    NodeKind::Macro,
                    parent_kind(node, &["macro_definition"]).unwrap_or(node),
                ),
                "definition.const" => (
                    NodeKind::Const,
                    parent_kind(node, &["const_item"]).unwrap_or(node),
                ),
                "definition.static" => (
                    NodeKind::Static,
                    parent_kind(node, &["static_item"]).unwrap_or(node),
                ),
                "definition.type" => (
                    NodeKind::TypeAlias,
                    parent_kind(node, &["type_item"]).unwrap_or(node),
                ),
                _ => continue,
            };

            let id = node_id(parsed, &def_node);
            symbols.push(ExtractedSymbol {
                id,
                kind,
                name,
                range: def_node.range(),
                parent_id: None,
                extra: Default::default(),
            });
        }

        symbols
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Walk up the tree to find a parent with one of the given kinds.
fn parent_kind<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if kinds.iter().any(|&k| parent.kind() == k) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

fn node_id(parsed: &ParsedFile, node: &Node) -> NodeId {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    parsed.file_path.hash(&mut hasher);
    node.start_position().row.hash(&mut hasher);
    node.start_position().column.hash(&mut hasher);
    node.kind().hash(&mut hasher);
    NodeId::new(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_rust(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.rs");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.rs"),
            size: code.len() as u64,
            language: Some(Language::Rust),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_functions() {
        let parsed = parse_rust(
            r#"
fn add(a: i32, b: i32) -> i32 { a + b }
fn main() { println!("hi"); }
"#,
        );
        let extractor = RustSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"main"));
        assert!(symbols.iter().any(|s| s.kind == NodeKind::Function));
    }

    #[test]
    fn extract_struct_and_impl() {
        let parsed = parse_rust(
            r#"
pub struct User { id: u64 }
impl User {
    pub fn new() -> Self { Self { id: 0 } }
}
"#,
        );
        let extractor = RustSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == NodeKind::Struct)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == NodeKind::Impl)
        );
    }

    #[test]
    fn extract_enum_trait() {
        let parsed = parse_rust(
            r#"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
"#,
        );
        let extractor = RustSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == NodeKind::Enum)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Drawable" && s.kind == NodeKind::Trait)
        );
    }

    #[test]
    fn symbol_and_call_id_match() {
        use crate::extractors::CallExtractor;
        let parsed = parse_rust(
            r#"
fn scan_directory() {}

fn scan_this_project() {
    scan_directory();
}
"#,
        );
        let extractor = RustSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        let calls = RustCallExtractor.extract(&parsed);

        let scan_this = symbols
            .iter()
            .find(|s| s.name == "scan_this_project")
            .unwrap();
        let call = calls
            .iter()
            .find(|c| c.callee_name == "scan_directory")
            .unwrap();

        assert_eq!(
            call.caller_id, scan_this.id,
            "caller_id should match the enclosing function's symbol id"
        );
    }
}

// ============================================================================
// Import Extractor
// ============================================================================

struct RustImportExtractor;

impl crate::extractors::ImportExtractor for RustImportExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedImport> {
        let mut imports = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();

        // Query to capture all use_declaration argument nodes
        let query_text = r#"
        (use_declaration
          argument: (_) @import.target) @import
        "#;
        let query = match build_query(grammar, query_text) {
            Ok(q) => q,
            Err(_) => return imports,
        };

        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, parsed.tree.root_node(), &parsed.source[..]);

        loop {
            let m = iter.next();
            if m.is_none() {
                break;
            }
            let m = m.unwrap();

            let mut target_node: Option<Node> = None;
            let mut import_node: Option<Node> = None;

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                if name == "import.target" {
                    target_node = Some(capture.node);
                } else if name == "import" {
                    import_node = Some(capture.node);
                }
            }

            let (Some(target), Some(import)) = (target_node, import_node) else {
                continue;
            };

            let range = import.range();
            let import_str = target
                .utf8_text(&parsed.source[..])
                .unwrap_or("")
                .to_string();

            let extracted = parse_use_tree(target, &import_str, range, parsed);
            imports.extend(extracted);
        }

        imports
    }
}

/// Parse a use_tree node into ExtractedImport(s).
fn parse_use_tree(
    node: Node,
    text: &str,
    range: tree_sitter::Range,
    parsed: &ParsedFile,
) -> Vec<crate::extractors::ExtractedImport> {
    use crate::extractors::{ExtractedImport, ImportedName};

    match node.kind() {
        "identifier" | "type_identifier" => {
            // `use Foo;` — simple single import
            vec![ExtractedImport {
                source: text.to_string(),
                names: vec![ImportedName {
                    local_name: text.to_string(),
                    original_name: None,
                }],
                range,
                is_wildcard: false,
            }]
        }
        "scoped_identifier" => {
            // `use foo::Bar;` or `use foo::bar::Baz;`
            // Extract the path and the final name
            let (path, name) = split_scoped_identifier(node, text);
            vec![ExtractedImport {
                source: path,
                names: vec![ImportedName {
                    local_name: name.clone(),
                    original_name: Some(name),
                }],
                range,
                is_wildcard: false,
            }]
        }
        "use_wildcard" => {
            // `use foo::*;`
            let path = text.trim_end_matches("::*").to_string();
            vec![ExtractedImport {
                source: path,
                names: vec![],
                range,
                is_wildcard: true,
            }]
        }
        "use_list" | "scoped_use_list" => {
            // `use foo::{Bar, Baz};` or `use {Bar, Baz};`
            let mut result = Vec::new();
            let mut path_prefix = String::new();
            let mut list_node = node;

            // For scoped_use_list, first child is the path prefix
            if node.kind() == "scoped_use_list"
                && let Some(first) = node.child(0)
            {
                path_prefix = first
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                // Find the actual use_list inside
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i)
                        && child.kind() == "use_list"
                    {
                        list_node = child;
                        break;
                    }
                }
            }

            // Iterate children of the use_list
            for i in 0..list_node.child_count() {
                let Some(child) = list_node.child(i) else {
                    continue;
                };
                let child_text = child
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();

                match child.kind() {
                    "identifier" | "type_identifier" => {
                        let full_path = if path_prefix.is_empty() {
                            child_text.clone()
                        } else {
                            format!("{}::{}", path_prefix, child_text)
                        };
                        result.push(ExtractedImport {
                            source: full_path,
                            names: vec![ImportedName {
                                local_name: child_text,
                                original_name: None,
                            }],
                            range: child.range(),
                            is_wildcard: false,
                        });
                    }
                    "use_as_clause" => {
                        // `Bar as Baz` inside a list
                        let (orig, alias) = parse_use_as_clause(child, parsed);
                        let full_path = if path_prefix.is_empty() {
                            orig.clone()
                        } else {
                            format!("{}::{}", path_prefix, orig)
                        };
                        result.push(ExtractedImport {
                            source: full_path,
                            names: vec![ImportedName {
                                local_name: alias,
                                original_name: Some(orig),
                            }],
                            range: child.range(),
                            is_wildcard: false,
                        });
                    }
                    "use_wildcard" => {
                        let path = if path_prefix.is_empty() {
                            child_text.trim_end_matches("::*").to_string()
                        } else {
                            format!("{}::*", path_prefix)
                        };
                        result.push(ExtractedImport {
                            source: path.trim_end_matches("::*").to_string(),
                            names: vec![],
                            range: child.range(),
                            is_wildcard: true,
                        });
                    }
                    _ => {}
                }
            }
            result
        }
        "use_as_clause" => {
            // `use foo::Bar as Baz;`
            let (path, alias) = parse_use_as_clause(node, parsed);
            let name = path.split("::").last().unwrap_or(&path).to_string();
            vec![ExtractedImport {
                source: path,
                names: vec![ImportedName {
                    local_name: alias,
                    original_name: Some(name),
                }],
                range,
                is_wildcard: false,
            }]
        }
        _ => Vec::new(),
    }
}

/// Split a scoped_identifier like `foo::bar::Baz` into path and name.
fn split_scoped_identifier(_node: Node, text: &str) -> (String, String) {
    // Split by :: — last component is the name, rest is the path
    if let Some(pos) = text.rfind("::") {
        let path = text[..pos].to_string();
        let name = text[pos + 2..].to_string();
        (path, name)
    } else {
        (String::new(), text.to_string())
    }
}

/// Parse a `use_as_clause` node: `(use_as_clause path: (_) name: (_) alias: (_))`
fn parse_use_as_clause(node: Node, parsed: &ParsedFile) -> (String, String) {
    let mut path = String::new();
    let mut alias = String::new();

    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        let text = child
            .utf8_text(&parsed.source[..])
            .unwrap_or("")
            .to_string();

        match child.kind() {
            "identifier" | "type_identifier" | "scoped_identifier" => {
                if path.is_empty() {
                    path = text;
                } else if alias.is_empty() {
                    alias = text;
                }
            }
            _ => {}
        }
    }

    // If alias is still empty, the last identifier is both path and alias
    if alias.is_empty() {
        alias = path.clone();
    }

    (path, alias)
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use crate::extractors::ImportExtractor;
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_rust(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.rs");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.rs"),
            size: code.len() as u64,
            language: Some(Language::Rust),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_simple_use() {
        let parsed = parse_rust("use std::collections::HashMap;\n");
        let extractor = RustImportExtractor;
        let imports = extractor.extract(&parsed);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].source, "std::collections");
        assert_eq!(imports[0].names[0].local_name, "HashMap");
    }

    #[test]
    fn extract_use_list() {
        let parsed = parse_rust("use std::collections::{HashMap, HashSet};\n");
        let extractor = RustImportExtractor;
        let imports = extractor.extract(&parsed);
        assert_eq!(imports.len(), 2);
        assert!(imports.iter().any(|i| i.names[0].local_name == "HashMap"));
        assert!(imports.iter().any(|i| i.names[0].local_name == "HashSet"));
    }

    #[test]
    fn extract_use_wildcard() {
        let parsed = parse_rust("use std::io::*;\n");
        let extractor = RustImportExtractor;
        let imports = extractor.extract(&parsed);
        assert_eq!(imports.len(), 1);
        assert!(imports[0].is_wildcard);
        assert_eq!(imports[0].source, "std::io");
    }
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct RustHeritageExtractor;

impl crate::extractors::HeritageExtractor for RustHeritageExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_rust_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_rust_heritage(
    parsed: &ParsedFile,
    node: Node,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    match node.kind() {
        "impl_item" => {
            // `impl Trait for Type` or `impl Type { ... }`
            if let Some(trait_node) = node.child_by_field_name("trait")
                && let Some(type_node) = node.child_by_field_name("type")
            {
                let trait_name = trait_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                let child_id = node_id(parsed, &type_node);
                heritages.push(crate::extractors::ExtractedHeritage {
                    child_id,
                    parent_name: trait_name,
                    kind: crate::extractors::HeritageKind::Implements,
                    range: node.range(),
                });
            }
        }
        "trait_item" => {
            // `trait A: B + C` (supertraits)
            if let Some(bounds_node) = node.child_by_field_name("bounds") {
                let child_id = node_id(parsed, &node);
                extract_trait_bounds(parsed, bounds_node, child_id, heritages);
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_rust_heritage(parsed, child, heritages);
        }
    }
}

fn extract_trait_bounds(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_identifier" | "scoped_type_identifier" => {
                    let name = child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                    if name != "+" && !name.is_empty() {
                        heritages.push(crate::extractors::ExtractedHeritage {
                            child_id,
                            parent_name: name,
                            kind: crate::extractors::HeritageKind::Extends,
                            range: child.range(),
                        });
                    }
                }
                _ => extract_trait_bounds(parsed, child, child_id, heritages),
            }
        }
    }
}

// ============================================================================
// Call Extractor
// ============================================================================

struct RustCallExtractor;

impl crate::extractors::CallExtractor for RustCallExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();

        // Note: tree-sitter-rust does NOT have `method_call_expression`.
        // Method calls are `call_expression` with `function: field_expression`.
        let query_text = r#"
        (call_expression
          function: (_) @call.function) @call

        (macro_invocation
          macro: (_) @call.function) @call
        "#;

        let query = match build_query(grammar, query_text) {
            Ok(q) => q,
            Err(_) => return calls,
        };

        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, parsed.tree.root_node(), &parsed.source[..]);

        loop {
            let m = iter.next();
            if m.is_none() {
                break;
            }
            let m = m.unwrap();

            let mut call_node: Option<Node> = None;
            let mut function_node: Option<Node> = None;

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                match name {
                    "call" => call_node = Some(capture.node),
                    "call.function" => function_node = Some(capture.node),
                    _ => {}
                }
            }

            let Some(call) = call_node else { continue };
            let Some(func) = function_node else { continue };
            let caller_id = find_enclosing_symbol_id(parsed, call);
            let range = call.range();
            let arg_count = count_arguments(call);

            let func_text = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            let (callee_name, call_form, receiver_name) = classify_call(func, func_text);

            calls.push(crate::extractors::ExtractedCall {
                caller_id,
                callee_name,
                call_form,
                range,
                receiver_name,
                argument_count: arg_count,
            });
        }

        calls
    }
}

/// Find the enclosing function/struct/impl/etc. and return its deterministic NodeId.
fn find_enclosing_symbol_id(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_item" | "method_item" | "closure_expression" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    // Fallback: use the call node itself as caller
    node_id(parsed, &node)
}

/// Count the number of arguments in a call expression.
fn count_arguments(call_node: Node) -> usize {
    for i in 0..call_node.child_count() {
        if let Some(child) = call_node.child(i)
            && (child.kind() == "arguments" || child.kind() == "token_tree")
        {
            // Count non-punctuation, non-whitespace children
            let mut count = 0;
            for j in 0..child.child_count() {
                if let Some(arg) = child.child(j)
                    && !matches!(arg.kind(), "," | "(" | ")" | "[" | "]" | "{")
                {
                    count += 1;
                }
            }
            return count;
        }
    }
    0
}

/// Classify a function call into Free / Member / Constructor.
fn classify_call(
    node: Node,
    text: String,
) -> (String, crate::extractors::CallForm, Option<String>) {
    match node.kind() {
        "identifier" => (text, crate::extractors::CallForm::Free, None),
        "scoped_identifier" => {
            let name = text.split("::").last().unwrap_or(&text).to_string();
            (name, crate::extractors::CallForm::Free, None)
        }
        "field_expression" => {
            // self.foo() or obj.foo() — split by last dot
            if let Some(pos) = text.rfind('.') {
                let receiver = text[..pos].to_string();
                let method = text[pos + 1..].to_string();
                (method, crate::extractors::CallForm::Member, Some(receiver))
            } else {
                (text, crate::extractors::CallForm::Free, None)
            }
        }
        _ => (text, crate::extractors::CallForm::Free, None),
    }
}

/// Basic Rust import path resolver.
///
/// Maps `crate::foo::bar` → `src/foo/bar.rs` or `src/foo/mod.rs`,
/// `super::foo` → parent dir + `foo.rs`,
/// `self::foo` → same dir + `foo.rs`.
fn resolve_rust_import(target: &str, from: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    // Skip standard library and external crates
    if target.starts_with("std::")
        || target.starts_with("core::")
        || target.starts_with("alloc::")
        || target.starts_with("proc_macro::")
        || !target.contains("::")
    {
        // For bare names like "crate_name", we can't resolve without Cargo.toml
        return None;
    }

    let mut parts: Vec<&str> = target.split("::").collect();

    // Determine base directory
    let base_dir = if parts[0] == "crate" {
        parts.remove(0);
        // Find the src directory by walking up from `from`
        find_src_root(from)
    } else if parts[0] == "super" {
        // Count super prefixes
        let mut super_count = 0;
        while !parts.is_empty() && parts[0] == "super" {
            parts.remove(0);
            super_count += 1;
        }
        let mut dir = from.parent()?;
        for _ in 0..super_count {
            dir = dir.parent()?;
        }
        dir.to_path_buf()
    } else if parts[0] == "self" {
        parts.remove(0);
        from.parent()?.to_path_buf()
    } else {
        // External crate or bare module — can't resolve without Cargo.lock
        return None;
    };

    // Try mapping the remaining path parts to a file
    if parts.is_empty() {
        return None;
    }

    // Build candidate paths:
    // e.g. `foo::bar` → `base_dir/foo/bar.rs` or `base_dir/foo/bar/mod.rs`
    let mut candidate = base_dir.clone();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last part: could be a file or a module
            let file_candidate = candidate.join(format!("{}.rs", part));
            if all_files.contains(&file_candidate) {
                return Some(file_candidate);
            }
            let mod_candidate = candidate.join(part).join("mod.rs");
            if all_files.contains(&mod_candidate) {
                return Some(mod_candidate);
            }
        } else {
            candidate = candidate.join(part);
        }
    }

    None
}

/// Walk up from a file to find the `src` directory (crate root).
fn find_src_root(from: &Path) -> PathBuf {
    let mut current = from;
    while let Some(parent) = current.parent() {
        if parent.file_name() == Some(std::ffi::OsStr::new("src")) {
            return parent.to_path_buf();
        }
        current = parent;
    }
    // Fallback: assume from is in src/
    from.parent().unwrap_or(from).to_path_buf()
}

#[cfg(test)]
mod call_tests {
    use super::*;
    use crate::extractors::CallExtractor;
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_rust(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.rs");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.rs"),
            size: code.len() as u64,
            language: Some(Language::Rust),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_free_function_call() {
        let parsed = parse_rust(
            r#"
fn main() {
    println!("hello");
    foo();
}
"#,
        );
        let extractor = RustCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"println"));
        assert!(names.contains(&"foo"));
    }

    #[test]
    fn extract_method_call() {
        let parsed = parse_rust(
            r#"
fn main() {
    let v = vec![1, 2, 3];
    v.push(4);
}
"#,
        );
        let extractor = RustCallExtractor;
        let calls = extractor.extract(&parsed);
        assert!(
            calls
                .iter()
                .any(|c| c.callee_name == "push" && c.receiver_name == Some("v".to_string()))
        );
    }

    #[test]
    fn extract_macro_invocation() {
        let parsed = parse_rust(
            r#"
fn main() {
    vec![1, 2, 3];
    assert_eq!(a, b);
}
"#,
        );
        let extractor = RustCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"vec"));
        assert!(names.contains(&"assert_eq"));
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn resolve_crate_import() {
        let mut all = HashSet::new();
        all.insert(PathBuf::from("/repo/src/models/user.rs"));
        all.insert(PathBuf::from("/repo/src/models/mod.rs"));

        let from = PathBuf::from("/repo/src/main.rs");
        assert_eq!(
            resolve_rust_import("crate::models::user", &from, &all),
            Some(PathBuf::from("/repo/src/models/user.rs"))
        );
    }

    #[test]
    fn resolve_super_import() {
        let mut all = HashSet::new();
        all.insert(PathBuf::from("/repo/src/utils/helper.rs"));

        let from = PathBuf::from("/repo/src/models/user.rs");
        assert_eq!(
            resolve_rust_import("super::utils::helper", &from, &all),
            Some(PathBuf::from("/repo/src/utils/helper.rs"))
        );
    }

    #[test]
    fn resolve_self_import() {
        let mut all = HashSet::new();
        all.insert(PathBuf::from("/repo/src/utils/helper.rs"));

        let from = PathBuf::from("/repo/src/utils/mod.rs");
        assert_eq!(
            resolve_rust_import("self::helper", &from, &all),
            Some(PathBuf::from("/repo/src/utils/helper.rs"))
        );
    }

    #[test]
    fn skip_std_import() {
        let all = HashSet::new();
        let from = PathBuf::from("/repo/src/main.rs");
        assert_eq!(
            resolve_rust_import("std::collections::HashMap", &from, &all),
            None
        );
    }
}

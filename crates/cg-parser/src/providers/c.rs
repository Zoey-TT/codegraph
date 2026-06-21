//! C language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{
    CallExtractor, CallForm, ExtractedCall, ExtractedImport, ExtractedSymbol, HeritageExtractor,
    ImportExtractor, SymbolExtractor,
};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

// ============================================================================
// Queries
// ============================================================================

const C_QUERIES: &str = r#"
;; Definitions
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @definition.function)) @def.function
(struct_specifier name: (type_identifier) @definition.struct) @def.struct
(union_specifier name: (type_identifier) @definition.union) @def.union
(enum_specifier name: (type_identifier) @definition.enum) @def.enum
(type_definition declarator: (type_identifier) @definition.typedef) @def.typedef
"#;

// ============================================================================
// CProvider
// ============================================================================

pub struct CProvider;

impl LanguageProvider for CProvider {
    fn id(&self) -> Language {
        Language::C
    }

    fn extensions(&self) -> &[&str] {
        &["c"]
    }

    fn tree_sitter_queries(&self) -> &str {
        C_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::WildcardTransitive
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::None
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(CSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn CallExtractor>> {
        Some(Box::new(CCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn ImportExtractor>> {
        Some(Box::new(CImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn HeritageExtractor>> {
        None
    }

    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        resolve_c_import(target, from, all_files)
    }

    fn is_exported(&self, _name: &str, _node: &Node, _source: &[u8]) -> bool {
        // C symbols are exported by default (header files handle visibility).
        true
    }
}

// ============================================================================
// Symbol Extractor
// ============================================================================

struct CSymbolExtractor;

impl SymbolExtractor for CSymbolExtractor {
    fn language(&self) -> Language {
        Language::C
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        extract_c_symbols(parsed, parsed.tree.root_node(), &mut symbols, None);
        symbols
    }
}

fn extract_c_symbols(
    parsed: &ParsedFile,
    node: Node,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_id: Option<NodeId>,
) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = find_child_recursive(node, "identifier") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        id: node_id(parsed, &node),
                        kind: NodeKind::Function,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                }
            }
        }
        "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        id: node_id(parsed, &node),
                        kind: NodeKind::Struct,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                }
            }
        }
        "union_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        id: node_id(parsed, &node),
                        kind: NodeKind::Union,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                }
            }
        }
        "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        id: node_id(parsed, &node),
                        kind: NodeKind::Enum,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                }
            }
        }
        "type_definition" => {
            // The typedef name is the type_identifier that is a direct child,
            // not the one inside a nested struct/enum specifier.
            let mut name = String::new();
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i)
                    && child.kind() == "type_identifier"
                {
                    name = child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                }
            }
            if !name.is_empty() {
                symbols.push(ExtractedSymbol {
                    id: node_id(parsed, &node),
                    kind: NodeKind::Typedef,
                    name,
                    range: node.range(),
                    parent_id,
                    extra: Default::default(),
                });
            }
        }
        "preproc_function_def" | "preproc_def" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        id: node_id(parsed, &node),
                        kind: NodeKind::Macro,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_c_symbols(parsed, child, symbols, parent_id);
        }
    }
}

// ============================================================================
// Call Extractor
// ============================================================================

struct CCallExtractor;

impl CallExtractor for CCallExtractor {
    fn language(&self) -> Language {
        Language::C
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();

        let query_text = r#"
        (call_expression
          function: (identifier) @call.function) @call
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
            let caller_id = find_enclosing_symbol_id_c(parsed, call);
            let range = call.range();
            let arg_count = count_arguments(call);
            let callee_name = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();

            calls.push(ExtractedCall {
                caller_id,
                callee_name,
                call_form: CallForm::Free,
                range,
                receiver_name: None,
                argument_count: arg_count,
            });
        }

        calls
    }
}

// ============================================================================
// Import Extractor
// ============================================================================

struct CImportExtractor;

impl ImportExtractor for CImportExtractor {
    fn language(&self) -> Language {
        Language::C
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedImport> {
        let mut imports = Vec::new();
        extract_c_imports_ast(parsed, parsed.tree.root_node(), &mut imports);
        imports
    }
}

fn extract_c_imports_ast(parsed: &ParsedFile, node: Node, imports: &mut Vec<ExtractedImport>) {
    if node.kind() == "preproc_include" {
        let mut path_node: Option<Node> = None;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "string_literal" | "system_lib_string" => {
                        path_node = Some(child);
                        break;
                    }
                    _ => {}
                }
            }
        }

        if let Some(path_node) = path_node {
            let raw = path_node
                .utf8_text(&parsed.source[..])
                .unwrap_or("")
                .to_string();
            let source = raw
                .trim_matches('"')
                .trim_matches('<')
                .trim_matches('>')
                .to_string();
            if !source.is_empty() {
                imports.push(ExtractedImport {
                    source,
                    names: Vec::new(),
                    range: node.range(),
                    is_wildcard: true,
                });
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_c_imports_ast(parsed, child, imports);
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn find_child_recursive<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
            if let Some(found) = find_child_recursive(child, kind) {
                return Some(found);
            }
        }
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

fn find_enclosing_symbol_id_c(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_definition" => return node_id(parsed, &parent),
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments(call_node: Node) -> usize {
    for i in 0..call_node.child_count() {
        if let Some(child) = call_node.child(i)
            && child.kind() == "argument_list"
        {
            let mut count = 0;
            for j in 0..child.child_count() {
                if let Some(arg) = child.child(j)
                    && !matches!(arg.kind(), "," | "(" | ")")
                {
                    count += 1;
                }
            }
            return count;
        }
    }
    0
}

fn resolve_c_import(target: &str, from: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    // System includes (<...>) can't be resolved to project files.
    if target.starts_with('/') || !target.contains('/') && !target.contains("\\") {
        // Heuristic: if the raw target looks like a system header (no path separators),
        // we still try to resolve it, but only if it was a quoted include.
        // The original include kind was lost by string extraction; we try both.
    }

    if let Some(base) = from.parent() {
        let resolved = base.join(target);
        let normalized = resolved.components().collect::<PathBuf>();
        if all_files.contains(&normalized) {
            return Some(normalized);
        }
    }

    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::{CallExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_c(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.c");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.c"),
            size: code.len() as u64,
            language: Some(Language::C),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_c_functions_and_structs() {
        let parsed = parse_c(
            r#"
#include <stdio.h>
#include "helper.h"

struct Point {
    int x;
    int y;
};

union Data {
    int i;
    float f;
};

enum Color { RED, GREEN, BLUE };

typedef struct Point PointAlias;

#define MAX(a,b) ((a)>(b)?(a):(b))
#define PI 3.14

void greet(void) {
    printf("hello");
}

int main(void) {
    greet();
    return 0;
}
"#,
        );
        let extractor = CSymbolExtractor;
        let symbols = extractor.extract(&parsed);

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "greet" && s.kind == NodeKind::Function),
            "Expected greet function"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "main" && s.kind == NodeKind::Function),
            "Expected main function"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Point" && s.kind == NodeKind::Struct),
            "Expected Point struct"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Data" && s.kind == NodeKind::Union),
            "Expected Data union"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == NodeKind::Enum),
            "Expected Color enum"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "PointAlias" && s.kind == NodeKind::Typedef),
            "Expected PointAlias typedef"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MAX" && s.kind == NodeKind::Macro),
            "Expected MAX macro"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "PI" && s.kind == NodeKind::Macro),
            "Expected PI macro"
        );
    }

    #[test]
    fn extract_c_calls() {
        let parsed = parse_c(
            r#"
void foo(void) {}

void bar(void) {
    foo();
}
"#,
        );
        let extractor = CCallExtractor;
        let calls = extractor.extract(&parsed);
        assert!(
            calls.iter().any(|c| c.callee_name == "foo"),
            "Expected foo call"
        );
    }

    #[test]
    fn extract_c_imports() {
        let parsed = parse_c(
            r#"
#include <stdio.h>
#include "helper.h"
#include "dir/utils.h"
"#,
        );
        let extractor = CImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "stdio.h" && i.is_wildcard),
            "Expected stdio.h include"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "helper.h" && i.is_wildcard),
            "Expected helper.h include"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "dir/utils.h" && i.is_wildcard),
            "Expected dir/utils.h include"
        );
    }

    #[test]
    fn c_is_exported() {
        let provider = CProvider;
        let parsed = parse_c("");
        assert!(provider.is_exported("foo", &parsed.tree.root_node(), b""));
    }

    #[test]
    fn c_resolve_import_quoted() {
        let provider = CProvider;
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("src/main.c");
        std::fs::create_dir_all(from.parent().unwrap()).unwrap();
        std::fs::write(&from, "").unwrap();

        let header = tmp.path().join("src/helper.h");
        std::fs::write(&header, "").unwrap();

        let mut all_files = HashSet::new();
        all_files.insert(header.clone());

        let resolved = provider.resolve_import("helper.h", &from, &all_files);
        assert_eq!(resolved, Some(header));
    }
}

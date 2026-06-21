//! Go language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{CallExtractor, HeritageExtractor, ImportExtractor, SymbolExtractor};
use crate::extractors::{
    CallForm, ExtractedCall, ExtractedHeritage, ExtractedImport, ExtractedSymbol, HeritageKind,
    ImportedName,
};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

// ============================================================================
// Queries
// ============================================================================

const GO_QUERIES: &str = r#"
;; Definitions
(function_declaration name: (identifier) @definition.function) @def.function
(method_declaration name: (field_identifier) @definition.method) @def.method
(type_spec name: (type_identifier) @definition.type) @def.type
(type_alias name: (type_identifier) @definition.type) @def.typealias
"#;

// ============================================================================
// GoProvider
// ============================================================================

pub struct GoProvider;

impl LanguageProvider for GoProvider {
    fn id(&self) -> Language {
        Language::Go
    }

    fn extensions(&self) -> &[&str] {
        &["go"]
    }

    fn tree_sitter_queries(&self) -> &str {
        GO_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::WildcardLeaf
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::None
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(GoSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn CallExtractor>> {
        Some(Box::new(GoCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn ImportExtractor>> {
        Some(Box::new(GoImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn HeritageExtractor>> {
        Some(Box::new(GoHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        resolve_go_import(target, from, all_files)
    }

    fn is_exported(&self, name: &str, _node: &Node, _source: &[u8]) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }
}

// ============================================================================
// Symbol Extractor
// ============================================================================

struct GoSymbolExtractor;

impl SymbolExtractor for GoSymbolExtractor {
    fn language(&self) -> Language {
        Language::Go
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        let query = match build_query(grammar, GO_QUERIES) {
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
                "definition.function" => {
                    let parent = parent_kind(node, &["function_declaration"]).unwrap_or(node);
                    (NodeKind::Function, parent)
                }
                "definition.method" => {
                    let parent = parent_kind(node, &["method_declaration"]).unwrap_or(node);
                    (NodeKind::Method, parent)
                }
                "definition.type" => {
                    let parent = parent_kind(node, &["type_spec", "type_alias"]).unwrap_or(node);
                    let kind = if parent.kind() == "type_alias" {
                        NodeKind::TypeAlias
                    } else if has_child_kind(parent, "struct_type") {
                        NodeKind::Struct
                    } else if has_child_kind(parent, "interface_type") {
                        NodeKind::Interface
                    } else {
                        NodeKind::TypeAlias
                    };
                    (kind, parent)
                }
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

fn has_child_kind(node: Node, kind: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == kind
        {
            return true;
        }
    }
    false
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

// ============================================================================
// Call Extractor
// ============================================================================

struct GoCallExtractor;

impl CallExtractor for GoCallExtractor {
    fn language(&self) -> Language {
        Language::Go
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();

        let query_text = r#"
        (call_expression
          function: (identifier) @call.function) @call

        (call_expression
          function: (selector_expression
            operand: (identifier) @call.object
            field: (field_identifier) @call.function)) @call
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
            let mut object_node: Option<Node> = None;

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                match name {
                    "call" => call_node = Some(capture.node),
                    "call.function" => function_node = Some(capture.node),
                    "call.object" => object_node = Some(capture.node),
                    _ => {}
                }
            }

            let Some(call) = call_node else { continue };
            let Some(func) = function_node else { continue };
            let caller_id = find_enclosing_symbol_id_go(parsed, call);
            let range = call.range();
            let arg_count = count_arguments_go(call);

            let callee_name = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            let (call_form, receiver_name) = if let Some(obj) = object_node {
                let receiver = obj.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
                (CallForm::Member, Some(receiver))
            } else {
                (CallForm::Free, None)
            };

            calls.push(ExtractedCall {
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

fn find_enclosing_symbol_id_go(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_declaration" | "method_declaration" | "func_literal" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments_go(call_node: Node) -> usize {
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

// ============================================================================
// Import Extractor
// ============================================================================

struct GoImportExtractor;

impl ImportExtractor for GoImportExtractor {
    fn language(&self) -> Language {
        Language::Go
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedImport> {
        let mut imports = Vec::new();
        extract_go_imports(parsed, parsed.tree.root_node(), &mut imports);
        imports
    }
}

fn extract_go_imports(parsed: &ParsedFile, node: Node, imports: &mut Vec<ExtractedImport>) {
    if node.kind() == "import_spec" {
        let range = node.range();
        let mut path = String::new();
        let mut alias: Option<String> = None;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "interpreted_string_literal" | "raw_string_literal" => {
                        path = child
                            .utf8_text(&parsed.source[..])
                            .unwrap_or("")
                            .trim_matches('"')
                            .trim_matches('`')
                            .to_string();
                    }
                    "package_identifier" => {
                        alias = Some(
                            child
                                .utf8_text(&parsed.source[..])
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                    _ => {}
                }
            }
        }

        let mut names = Vec::new();
        if let Some(alias_name) = alias {
            names.push(ImportedName {
                local_name: alias_name,
                original_name: None,
            });
        }

        imports.push(ExtractedImport {
            source: path,
            names,
            range,
            is_wildcard: true,
        });
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_go_imports(parsed, child, imports);
        }
    }
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct GoHeritageExtractor;

impl HeritageExtractor for GoHeritageExtractor {
    fn language(&self) -> Language {
        Language::Go
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_go_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_go_heritage(parsed: &ParsedFile, node: Node, heritages: &mut Vec<ExtractedHeritage>) {
    if node.kind() == "type_spec" {
        // Only interfaces can explicitly embed other types in Go.
        if has_child_kind(node, "interface_type") {
            let child_id = node_id(parsed, &node);
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i)
                    && child.kind() == "interface_type"
                {
                    extract_interface_embeds(parsed, child, child_id, heritages);
                }
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_go_heritage(parsed, child, heritages);
        }
    }
}

fn extract_interface_embeds(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    heritages: &mut Vec<ExtractedHeritage>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_elem" => {
                    extract_type_elem_embeds(parsed, child, child_id, heritages);
                }
                _ => extract_interface_embeds(parsed, child, child_id, heritages),
            }
        }
    }
}

fn extract_type_elem_embeds(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    heritages: &mut Vec<ExtractedHeritage>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "type_identifier" {
                let name = child
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    heritages.push(ExtractedHeritage {
                        child_id,
                        parent_name: name,
                        kind: HeritageKind::Extends,
                        range: child.range(),
                    });
                }
            } else {
                extract_type_elem_embeds(parsed, child, child_id, heritages);
            }
        }
    }
}

// ============================================================================
// Import Resolution
// ============================================================================

fn resolve_go_import(target: &str, from: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    // Relative imports: resolve against the importing file's directory.
    if target.starts_with("./") || target.starts_with("../") {
        if let Some(base) = from.parent() {
            let dir = normalize_path(&base.join(target));
            return find_go_file_in_dir(&dir, all_files);
        }
        return None;
    }

    // Module imports: treat the import path as a directory and look for any .go file inside it.
    let target_path = PathBuf::from(target);
    find_go_file_in_dir(&target_path, all_files)
}

fn find_go_file_in_dir(dir: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    for file in all_files {
        if file.extension().map(|e| e == "go").unwrap_or(false)
            && let Some(parent) = file.parent()
            && parent == dir
        {
            return Some(file.clone());
        }
    }
    None
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::{CallExtractor, HeritageExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_go(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.go");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.go"),
            size: code.len() as u64,
            language: Some(Language::Go),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_go_symbols() {
        let parsed = parse_go(
            r#"
package main

func Hello() {}
func (r *Receiver) Method() {}

type MyStruct struct {
    Name string
}

type MyInterface interface {
    Do()
}

type MyAlias = int
"#,
        );
        let extractor = GoSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Hello" && s.kind == NodeKind::Function),
            "Expected Hello function"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Method" && s.kind == NodeKind::Method),
            "Expected Method method"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyStruct" && s.kind == NodeKind::Struct),
            "Expected MyStruct struct"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyInterface" && s.kind == NodeKind::Interface),
            "Expected MyInterface interface"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyAlias" && s.kind == NodeKind::TypeAlias),
            "Expected MyAlias type alias"
        );
    }

    #[test]
    fn extract_go_calls() {
        let parsed = parse_go(
            r#"
package main

func Hello() {}
func main() {
    Hello()
    fmt.Println("hi")
}
"#,
        );
        let extractor = GoCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(
            names.contains(&"Hello"),
            "Expected Hello call, got {:?}",
            names
        );
        assert!(
            names.contains(&"Println"),
            "Expected Println call, got {:?}",
            names
        );

        let member = calls.iter().find(|c| c.callee_name == "Println").unwrap();
        assert_eq!(member.receiver_name, Some("fmt".to_string()));
        assert_eq!(member.call_form, CallForm::Member);

        let free = calls.iter().find(|c| c.callee_name == "Hello").unwrap();
        assert_eq!(free.receiver_name, None);
        assert_eq!(free.call_form, CallForm::Free);
    }

    #[test]
    fn extract_go_imports() {
        let parsed = parse_go(
            r#"
package main

import "fmt"
import (
    "strings"
    alias "path/to/pkg"
)
"#,
        );
        let extractor = GoImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports.iter().any(|i| i.source == "fmt" && i.is_wildcard),
            "Expected fmt import"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "strings" && i.is_wildcard),
            "Expected strings import"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "path/to/pkg"
                    && i.names.iter().any(|n| n.local_name == "alias")),
            "Expected aliased import"
        );
    }

    #[test]
    fn extract_go_heritage() {
        let parsed = parse_go(
            r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
}

type ReadCloser interface {
    Reader
    Close() error
}
"#,
        );
        let extractor = GoHeritageExtractor;
        let heritages = extractor.extract(&parsed);
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Reader" && h.kind == HeritageKind::Extends),
            "Expected heritage Reader"
        );
    }

    #[test]
    fn go_is_exported() {
        let provider = GoProvider;
        let parsed = parse_go("");
        assert!(provider.is_exported("Hello", &parsed.tree.root_node(), b""));
        assert!(!provider.is_exported("hello", &parsed.tree.root_node(), b""));
        assert!(!provider.is_exported("_Hello", &parsed.tree.root_node(), b""));
    }

    #[test]
    fn resolve_go_import_as_directory() {
        let mut all_files = HashSet::new();
        all_files.insert(PathBuf::from("github.com/user/repo/pkg/foo.go"));
        all_files.insert(PathBuf::from("github.com/user/repo/pkg/bar.go"));

        let provider = GoProvider;
        let from = PathBuf::from("github.com/user/repo/main.go");

        let resolved = provider.resolve_import("github.com/user/repo/pkg", &from, &all_files);
        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert!(
            resolved == *"github.com/user/repo/pkg/foo.go"
                || resolved == *"github.com/user/repo/pkg/bar.go"
        );
    }
}

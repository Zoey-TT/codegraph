//! TypeScript / JavaScript language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{ExtractedSymbol, SymbolExtractor};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

const TS_QUERIES: &str = r#"
;; Definitions
(function_declaration name: (identifier) @definition.function) @def.function
(class_declaration name: (type_identifier) @definition.class) @def.class
(interface_declaration name: (type_identifier) @definition.interface) @def.interface
(method_definition name: (property_identifier) @definition.method) @def.method
(type_alias_declaration name: (type_identifier) @definition.type) @def.typealias
(enum_declaration name: (identifier) @definition.enum) @def.enum
"#;

pub struct TypeScriptProvider;

impl LanguageProvider for TypeScriptProvider {
    fn id(&self) -> Language {
        Language::TypeScript
    }

    fn extensions(&self) -> &[&str] {
        &["ts", "tsx", "mts", "cts"]
    }

    fn tree_sitter_queries(&self) -> &str {
        TS_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::Named
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::FirstWins
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(TsSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn crate::extractors::CallExtractor>> {
        Some(Box::new(TsCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn crate::extractors::ImportExtractor>> {
        Some(Box::new(TsImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn crate::extractors::HeritageExtractor>> {
        Some(Box::new(TsHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        resolve_ts_import(target, from, all_files)
    }

    fn is_exported(&self, _name: &str, node: &Node, _source: &[u8]) -> bool {
        let mut current = *node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "export_statement" {
                return true;
            }
            if [
                "function_declaration",
                "class_declaration",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
            ]
            .contains(&parent.kind())
            {
                for i in 0..parent.child_count() {
                    if let Some(child) = parent.child(i)
                        && child.kind() == "export"
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

struct TsSymbolExtractor;

impl SymbolExtractor for TsSymbolExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = match build_query(grammar, TS_QUERIES) {
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
                    parent_kind(
                        node,
                        &[
                            "function_declaration",
                            "function_expression",
                            "arrow_function",
                        ],
                    )
                    .unwrap_or(node),
                ),
                "definition.class" => (
                    NodeKind::Class,
                    parent_kind(node, &["class_declaration", "class_expression"]).unwrap_or(node),
                ),
                "definition.interface" => (
                    NodeKind::Interface,
                    parent_kind(node, &["interface_declaration"]).unwrap_or(node),
                ),
                "definition.method" => (
                    NodeKind::Method,
                    parent_kind(node, &["method_definition"]).unwrap_or(node),
                ),
                "definition.type" => (
                    NodeKind::TypeAlias,
                    parent_kind(node, &["type_alias_declaration"]).unwrap_or(node),
                ),
                "definition.enum" => (
                    NodeKind::Enum,
                    parent_kind(node, &["enum_declaration"]).unwrap_or(node),
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

fn resolve_ts_import(target: &str, from: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
    if target.starts_with('.') {
        let base = from.parent()?;
        let candidates = [
            format!("{}.ts", target),
            format!("{}.tsx", target),
            format!("{}.js", target),
            format!("{}/index.ts", target),
            format!("{}/index.tsx", target),
            format!("{}/index.js", target),
        ];
        for cand in &candidates {
            let resolved = base.join(cand);
            let normalized = resolved.components().collect::<PathBuf>();
            if all_files.contains(&normalized) {
                return Some(normalized);
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

// ============================================================================
// Call Extractor
// ============================================================================

struct TsCallExtractor;

impl crate::extractors::CallExtractor for TsCallExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();

        let query_text = r#"
        (call_expression function: (_) @call.function) @call
        (new_expression constructor: (_) @call.function) @call
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
            let caller_id = find_enclosing_symbol_id_ts(parsed, call);
            let range = call.range();
            let arg_count = count_arguments_ts(call);
            let is_new = call.kind() == "new_expression";

            let (callee_name, call_form, receiver_name) = classify_ts_call(func, parsed);

            calls.push(crate::extractors::ExtractedCall {
                caller_id,
                callee_name,
                call_form: if is_new {
                    crate::extractors::CallForm::Constructor
                } else {
                    call_form
                },
                range,
                receiver_name,
                argument_count: arg_count,
            });
        }

        calls
    }
}

fn find_enclosing_symbol_id_ts(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "function_expression" => {
                return node_id(parsed, &parent);
            }
            "class_declaration" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments_ts(call_node: Node) -> usize {
    for i in 0..call_node.child_count() {
        if let Some(child) = call_node.child(i)
            && child.kind() == "arguments"
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

fn classify_ts_call(
    func: Node,
    parsed: &ParsedFile,
) -> (String, crate::extractors::CallForm, Option<String>) {
    match func.kind() {
        "identifier" => {
            let name = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            (name, crate::extractors::CallForm::Free, None)
        }
        "member_expression" => {
            let text = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            if let Some(pos) = text.rfind('.') {
                let receiver = text[..pos].to_string();
                let method = text[pos + 1..].to_string();
                (method, crate::extractors::CallForm::Member, Some(receiver))
            } else {
                (text, crate::extractors::CallForm::Free, None)
            }
        }
        "call_expression" => {
            let text = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            (text, crate::extractors::CallForm::Free, None)
        }
        _ => {
            let text = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            (text, crate::extractors::CallForm::Free, None)
        }
    }
}

// ============================================================================
// Import Extractor
// ============================================================================

struct TsImportExtractor;

impl crate::extractors::ImportExtractor for TsImportExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedImport> {
        let mut imports = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();

        let query_text = r#"
        (import_statement
          source: (string) @import.source) @import

        (export_statement
          source: (string) @import.source) @import
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

            let mut source_node: Option<Node> = None;
            let mut import_node: Option<Node> = None;

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                match name {
                    "import.source" => source_node = Some(capture.node),
                    "import" => import_node = Some(capture.node),
                    _ => {}
                }
            }

            let (Some(source), Some(import)) = (source_node, import_node) else {
                continue;
            };

            let range = import.range();
            let source_str = source
                .utf8_text(&parsed.source[..])
                .unwrap_or("")
                .to_string();
            let source_path = source_str
                .trim_matches(|c| c == '"' || c == '\'' || c == '`')
                .to_string();

            let mut names = Vec::new();
            let mut is_wildcard = false;

            for i in 0..import.child_count() {
                if let Some(child) = import.child(i)
                    && child.kind() == "import_clause"
                {
                    for j in 0..child.child_count() {
                        if let Some(clause_child) = child.child(j) {
                            match clause_child.kind() {
                                "named_imports" => {
                                    for k in 0..clause_child.child_count() {
                                        if let Some(spec) = clause_child.child(k)
                                            && spec.kind() == "import_specifier"
                                        {
                                            let (local, orig) =
                                                parse_ts_import_specifier(spec, parsed);
                                            names.push(crate::extractors::ImportedName {
                                                local_name: local,
                                                original_name: Some(orig),
                                            });
                                        }
                                    }
                                }
                                "identifier" | "type_identifier" => {
                                    let name = clause_child
                                        .utf8_text(&parsed.source[..])
                                        .unwrap_or("")
                                        .to_string();
                                    names.push(crate::extractors::ImportedName {
                                        local_name: name.clone(),
                                        original_name: Some(name),
                                    });
                                }
                                "namespace_import" => {
                                    is_wildcard = true;
                                    for k in 0..clause_child.child_count() {
                                        if let Some(id) = clause_child.child(k)
                                            && matches!(id.kind(), "identifier" | "type_identifier")
                                        {
                                            let name = id
                                                .utf8_text(&parsed.source[..])
                                                .unwrap_or("")
                                                .to_string();
                                            names.push(crate::extractors::ImportedName {
                                                local_name: name,
                                                original_name: None,
                                            });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            imports.push(crate::extractors::ExtractedImport {
                source: source_path,
                names,
                range,
                is_wildcard,
            });
        }

        imports
    }
}

fn parse_ts_import_specifier(node: Node, parsed: &ParsedFile) -> (String, String) {
    let mut local = String::new();
    let mut orig = String::new();

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let text = child
                .utf8_text(&parsed.source[..])
                .unwrap_or("")
                .to_string();
            match child.kind() {
                "identifier" | "type_identifier" => {
                    if orig.is_empty() {
                        orig = text.clone();
                        local = text;
                    } else {
                        local = text;
                    }
                }
                _ => {}
            }
        }
    }

    if local.is_empty() {
        local = orig.clone();
    }

    (local, orig)
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct TsHeritageExtractor;

impl crate::extractors::HeritageExtractor for TsHeritageExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_ts_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_ts_heritage(
    parsed: &ParsedFile,
    node: Node,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    match node.kind() {
        "class_declaration" | "interface_declaration" => {
            let child_id = node_id(parsed, &node);
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    match child.kind() {
                        "extends_clause" | "implements_clause" | "interface_heritage" => {
                            let kind = if child.kind() == "implements_clause" {
                                crate::extractors::HeritageKind::Implements
                            } else {
                                crate::extractors::HeritageKind::Extends
                            };
                            extract_heritage_clause(parsed, child, child_id, kind, heritages);
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_ts_heritage(parsed, child, heritages);
                }
            }
        }
    }
}

fn extract_heritage_clause(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    kind: crate::extractors::HeritageKind,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_identifier" | "identifier" | "member_expression" => {
                    let name = child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                    if name != "extends" && name != "implements" && !name.is_empty() {
                        heritages.push(crate::extractors::ExtractedHeritage {
                            child_id,
                            parent_name: name,
                            kind,
                            range: child.range(),
                        });
                    }
                }
                _ => extract_heritage_clause(parsed, child, child_id, kind, heritages),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::{CallExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_ts(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.ts");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.ts"),
            size: code.len() as u64,
            language: Some(Language::TypeScript),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_ts_functions_and_classes() {
        let parsed = parse_ts(
            r#"
function greet(name: string): string { return name; }
class User {
    id: number;
    constructor(id: number) { this.id = id; }
    getName(): string { return "user"; }
}
interface Named { name: string; }
"#,
        );
        let extractor = TsSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "greet" && s.kind == NodeKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == NodeKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "getName" && s.kind == NodeKind::Method)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Named" && s.kind == NodeKind::Interface)
        );
    }

    #[test]
    fn extract_ts_calls() {
        let parsed = parse_ts(
            r#"
function foo() {}
class Bar {
    baz() {}
}
function main() {
    foo();
    const b = new Bar();
    b.baz();
    console.log("hi");
}
"#,
        );
        let extractor = TsCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"foo"), "Expected foo call, got {:?}", names);
        assert!(
            names.contains(&"Bar"),
            "Expected Bar constructor, got {:?}",
            names
        );
        assert!(
            names.contains(&"baz"),
            "Expected baz method, got {:?}",
            names
        );
        assert!(
            names.contains(&"log"),
            "Expected log method, got {:?}",
            names
        );

        let member = calls.iter().find(|c| c.callee_name == "baz").unwrap();
        assert_eq!(member.receiver_name, Some("b".to_string()));
        assert_eq!(member.call_form, crate::extractors::CallForm::Member);
    }

    #[test]
    fn extract_ts_imports() {
        let parsed = parse_ts(
            r#"
import { foo, bar as baz } from "./module";
import * as utils from "utils";
import React from "react";
"#,
        );
        let extractor = TsImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "./module" && i.names.iter().any(|n| n.local_name == "foo")),
            "Expected named import foo"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "./module" && i.names.iter().any(|n| n.local_name == "baz")),
            "Expected aliased import baz"
        );
        assert!(
            imports.iter().any(|i| i.source == "utils" && i.is_wildcard),
            "Expected wildcard import from utils"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "react" && i.names.iter().any(|n| n.local_name == "React")),
            "Expected default import React"
        );
    }
}

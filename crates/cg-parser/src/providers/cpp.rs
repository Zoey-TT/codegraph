//! C++ language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{
    CallExtractor, CallForm, ExtractedCall, ExtractedHeritage, ExtractedImport, ExtractedSymbol,
    HeritageExtractor, HeritageKind, ImportExtractor, SymbolExtractor,
};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

// ============================================================================
// Queries
// ============================================================================

const CPP_QUERIES: &str = r#"
;; Definitions
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @definition.function)) @def.function
(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @definition.method)) @def.method
(class_specifier name: (type_identifier) @definition.class) @def.class
(struct_specifier name: (type_identifier) @definition.struct) @def.struct
(namespace_definition name: (identifier) @definition.namespace) @def.namespace
"#;

// ============================================================================
// CppProvider
// ============================================================================

pub struct CppProvider;

impl LanguageProvider for CppProvider {
    fn id(&self) -> Language {
        Language::Cpp
    }

    fn extensions(&self) -> &[&str] {
        &["cpp", "cc", "cxx", "hpp", "h"]
    }

    fn tree_sitter_queries(&self) -> &str {
        CPP_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::WildcardTransitive
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::FirstWins
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(CppSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn CallExtractor>> {
        Some(Box::new(CppCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn ImportExtractor>> {
        Some(Box::new(CppImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn HeritageExtractor>> {
        Some(Box::new(CppHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        resolve_cpp_import(target, from, all_files)
    }

    fn is_exported(&self, _name: &str, _node: &Node, _source: &[u8]) -> bool {
        // C++ symbols are exported by default (headers / visibility attributes not tracked).
        true
    }
}

// ============================================================================
// Symbol Extractor
// ============================================================================

struct CppSymbolExtractor;

impl SymbolExtractor for CppSymbolExtractor {
    fn language(&self) -> Language {
        Language::Cpp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        extract_cpp_symbols(parsed, parsed.tree.root_node(), &mut symbols, None);
        symbols
    }
}

fn extract_cpp_symbols(
    parsed: &ParsedFile,
    node: Node,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_id: Option<NodeId>,
) {
    match node.kind() {
        "function_definition" => {
            // Free function or method: identifier/field_identifier inside function_declarator
            if let Some(declarator) = node.child_by_field_name("declarator")
                && let Some(name_node) = find_declarator_name(declarator)
            {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    let kind = if name_node.kind() == "field_identifier" {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
                    };
                    let id = node_id(parsed, &node);
                    symbols.push(ExtractedSymbol {
                        id,
                        kind,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                    // Descend into children with this function as parent scope
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            extract_cpp_symbols(parsed, child, symbols, Some(id));
                        }
                    }
                    return;
                }
            }
        }
        "class_specifier" | "struct_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    let id = node_id(parsed, &node);
                    symbols.push(ExtractedSymbol {
                        id,
                        kind: NodeKind::Class,
                        name,
                        range: node.range(),
                        parent_id,
                        extra: Default::default(),
                    });
                    // Descend into children with this class as parent scope
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            extract_cpp_symbols(parsed, child, symbols, Some(id));
                        }
                    }
                    return;
                }
            }
        }
        "namespace_definition" => {
            let name = extract_namespace_name(parsed, node);
            if !name.is_empty() {
                let id = node_id(parsed, &node);
                symbols.push(ExtractedSymbol {
                    id,
                    kind: NodeKind::Namespace,
                    name,
                    range: node.range(),
                    parent_id,
                    extra: Default::default(),
                });
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        extract_cpp_symbols(parsed, child, symbols, Some(id));
                    }
                }
                return;
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_cpp_symbols(parsed, child, symbols, parent_id);
        }
    }
}

fn find_declarator_name(node: Node) -> Option<Node> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return Some(node);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && let Some(found) = find_declarator_name(child)
        {
            return Some(found);
        }
    }
    None
}

fn extract_namespace_name(parsed: &ParsedFile, node: Node) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "identifier" | "namespace_identifier" => {
                    return child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                }
                "nested_namespace_specifier" => {
                    return child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                }
                _ => {}
            }
        }
    }
    String::new()
}

// ============================================================================
// Call Extractor
// ============================================================================

struct CppCallExtractor;

impl CallExtractor for CppCallExtractor {
    fn language(&self) -> Language {
        Language::Cpp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();

        let query_text = r#"
        (call_expression
          function: (identifier) @call.function) @call

        (call_expression
          function: (field_expression
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
                    _ => {}
                }
            }

            // For member calls, the function_node is the field_identifier inside field_expression.
            // Try to find the object (receiver) from the call's function node parent.
            if let Some(_call_n) = call_node
                && let Some(func_n) = function_node
                && let Some(parent) = func_n.parent()
                && parent.kind() == "field_expression"
                && let Some(object) = parent.child_by_field_name("argument")
            {
                object_node = Some(object);
            }

            let Some(call) = call_node else { continue };
            let Some(func) = function_node else { continue };
            let caller_id = find_enclosing_symbol_id_cpp(parsed, call);
            let range = call.range();
            let arg_count = count_arguments(call);
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

// ============================================================================
// Import Extractor
// ============================================================================

struct CppImportExtractor;

impl ImportExtractor for CppImportExtractor {
    fn language(&self) -> Language {
        Language::Cpp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedImport> {
        let mut imports = Vec::new();
        extract_cpp_imports_ast(parsed, parsed.tree.root_node(), &mut imports);
        imports
    }
}

fn extract_cpp_imports_ast(parsed: &ParsedFile, node: Node, imports: &mut Vec<ExtractedImport>) {
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
            extract_cpp_imports_ast(parsed, child, imports);
        }
    }
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct CppHeritageExtractor;

impl HeritageExtractor for CppHeritageExtractor {
    fn language(&self) -> Language {
        Language::Cpp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_cpp_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_cpp_heritage(parsed: &ParsedFile, node: Node, heritages: &mut Vec<ExtractedHeritage>) {
    if node.kind() == "class_specifier" || node.kind() == "struct_specifier" {
        let child_id = node_id(parsed, &node);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i)
                && child.kind() == "base_class_clause"
            {
                extract_base_classes(parsed, child, child_id, heritages);
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_cpp_heritage(parsed, child, heritages);
        }
    }
}

fn extract_base_classes(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    heritages: &mut Vec<ExtractedHeritage>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_identifier" => {
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
                }
                _ => extract_base_classes(parsed, child, child_id, heritages),
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

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

fn find_enclosing_symbol_id_cpp(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_definition"
            | "class_specifier"
            | "struct_specifier"
            | "namespace_definition" => {
                return node_id(parsed, &parent);
            }
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

fn resolve_cpp_import(target: &str, from: &Path, all_files: &HashSet<PathBuf>) -> Option<PathBuf> {
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
    use crate::extractors::{CallExtractor, HeritageExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_cpp(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cpp");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.cpp"),
            size: code.len() as u64,
            language: Some(Language::Cpp),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_cpp_classes_functions_namespaces() {
        let parsed = parse_cpp(
            r#"
#include <iostream>
#include "helper.hpp"

namespace my {
    namespace inner {
        void ns_func() {}
    }
}

class Animal {
public:
    void speak() {}
};

class Dog : public Animal {
public:
    void bark() {}
};

struct Point {
    int x;
    int y;
};

void free_func() {
    Dog d;
    d.bark();
}
"#,
        );
        let extractor = CppSymbolExtractor;
        let symbols = extractor.extract(&parsed);

        assert!(
            symbols
                .iter()
                .any(|s| s.name == "my" && s.kind == NodeKind::Namespace),
            "Expected my namespace"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "inner" && s.kind == NodeKind::Namespace),
            "Expected inner namespace"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Animal" && s.kind == NodeKind::Class),
            "Expected Animal class"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Dog" && s.kind == NodeKind::Class),
            "Expected Dog class"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Point" && s.kind == NodeKind::Class),
            "Expected Point struct"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "free_func" && s.kind == NodeKind::Function),
            "Expected free_func"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "ns_func" && s.kind == NodeKind::Function),
            "Expected ns_func"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "speak" && s.kind == NodeKind::Method),
            "Expected speak method"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "bark" && s.kind == NodeKind::Method),
            "Expected bark method"
        );
    }

    #[test]
    fn extract_cpp_calls() {
        let parsed = parse_cpp(
            r#"
void foo() {}

void bar() {
    foo();
    int x = 1;
}

class C {
public:
    void method() {}
};

void baz() {
    C c;
    c.method();
}
"#,
        );
        let extractor = CppCallExtractor;
        let calls = extractor.extract(&parsed);

        assert!(
            calls
                .iter()
                .any(|c| c.callee_name == "foo" && c.call_form == CallForm::Free),
            "Expected foo call"
        );

        let member = calls.iter().find(|c| c.callee_name == "method").unwrap();
        assert_eq!(member.call_form, CallForm::Member);
        assert_eq!(member.receiver_name, Some("c".to_string()));
    }

    #[test]
    fn extract_cpp_imports() {
        let parsed = parse_cpp(
            r#"
#include <vector>
#include "utils.hpp"
"#,
        );
        let extractor = CppImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "vector" && i.is_wildcard),
            "Expected vector include"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "utils.hpp" && i.is_wildcard),
            "Expected utils.hpp include"
        );
    }

    #[test]
    fn extract_cpp_heritage() {
        let parsed = parse_cpp(
            r#"
class Base {};
class Derived : public Base {};
"#,
        );
        let extractor = CppHeritageExtractor;
        let heritages = extractor.extract(&parsed);
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Base" && h.kind == HeritageKind::Extends),
            "Expected extends Base"
        );
    }

    #[test]
    fn cpp_is_exported() {
        let provider = CppProvider;
        let parsed = parse_cpp("");
        assert!(provider.is_exported("foo", &parsed.tree.root_node(), b""));
    }

    #[test]
    fn cpp_resolve_import() {
        let provider = CppProvider;
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("src/main.cpp");
        std::fs::create_dir_all(from.parent().unwrap()).unwrap();
        std::fs::write(&from, "").unwrap();

        let header = tmp.path().join("src/helper.hpp");
        std::fs::write(&header, "").unwrap();

        let mut all_files = HashSet::new();
        all_files.insert(header.clone());

        let resolved = provider.resolve_import("helper.hpp", &from, &all_files);
        assert_eq!(resolved, Some(header));
    }
}

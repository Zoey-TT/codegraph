//! Python language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{ExtractedSymbol, SymbolExtractor};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

const PYTHON_QUERIES: &str = r#"
;; Definitions
(function_definition name: (identifier) @definition.function) @def.function
(class_definition name: (identifier) @definition.class) @def.class
"#;

pub struct PythonProvider;

impl LanguageProvider for PythonProvider {
    fn id(&self) -> Language {
        Language::Python
    }

    fn extensions(&self) -> &[&str] {
        &["py", "pyi"]
    }

    fn tree_sitter_queries(&self) -> &str {
        PYTHON_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::Named
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::C3
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(PythonSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn crate::extractors::CallExtractor>> {
        Some(Box::new(PythonCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn crate::extractors::ImportExtractor>> {
        Some(Box::new(PythonImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn crate::extractors::HeritageExtractor>> {
        Some(Box::new(PythonHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        _from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        let target_path = target.replace('.', "/");
        let candidates = [
            PathBuf::from(&target_path).with_extension("py"),
            PathBuf::from(&target_path).join("__init__.py"),
        ];
        for cand in &candidates {
            if all_files.contains(cand) {
                return Some(cand.clone());
            }
        }
        None
    }

    fn is_exported(&self, _name: &str, _node: &Node, _source: &[u8]) -> bool {
        true
    }
}

struct PythonSymbolExtractor;

impl SymbolExtractor for PythonSymbolExtractor {
    fn language(&self) -> Language {
        Language::Python
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        let query = match build_query(grammar, PYTHON_QUERIES) {
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
                    parent_kind(node, &["function_definition"]).unwrap_or(node),
                ),
                "definition.class" => (
                    NodeKind::Class,
                    parent_kind(node, &["class_definition"]).unwrap_or(node),
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

struct PythonCallExtractor;

impl crate::extractors::CallExtractor for PythonCallExtractor {
    fn language(&self) -> Language {
        Language::Python
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();

        let query_text = r#"
        (call function: (_) @call.function) @call
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
            let caller_id = find_enclosing_symbol_id_py(parsed, call);
            let range = call.range();
            let arg_count = count_arguments_py(call);

            let (callee_name, call_form, receiver_name) = classify_py_call(func, parsed);

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

fn find_enclosing_symbol_id_py(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "function_definition" | "lambda" => {
                return node_id(parsed, &parent);
            }
            "class_definition" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments_py(call_node: Node) -> usize {
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

fn classify_py_call(
    func: Node,
    parsed: &ParsedFile,
) -> (String, crate::extractors::CallForm, Option<String>) {
    match func.kind() {
        "identifier" => {
            let name = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            (name, crate::extractors::CallForm::Free, None)
        }
        "attribute" => {
            let text = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            if let Some(pos) = text.rfind('.') {
                let receiver = text[..pos].to_string();
                let method = text[pos + 1..].to_string();
                (method, crate::extractors::CallForm::Member, Some(receiver))
            } else {
                (text, crate::extractors::CallForm::Free, None)
            }
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

struct PythonImportExtractor;

impl crate::extractors::ImportExtractor for PythonImportExtractor {
    fn language(&self) -> Language {
        Language::Python
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedImport> {
        let mut imports = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();

        let query_text = r#"
        (import_statement) @import
        (import_from_statement) @import
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

            let import_node = m.captures[0].node;
            let range = import_node.range();

            match import_node.kind() {
                "import_statement" => {
                    for i in 0..import_node.child_count() {
                        if let Some(child) = import_node.child(i)
                            && child.kind() == "dotted_name"
                        {
                            let name = child
                                .utf8_text(&parsed.source[..])
                                .unwrap_or("")
                                .to_string();
                            imports.push(crate::extractors::ExtractedImport {
                                source: name.clone(),
                                names: vec![crate::extractors::ImportedName {
                                    local_name: name
                                        .split('.')
                                        .next_back()
                                        .unwrap_or(&name)
                                        .to_string(),
                                    original_name: Some(name.clone()),
                                }],
                                range,
                                is_wildcard: false,
                            });
                        }
                    }
                }
                "import_from_statement" => {
                    let mut source = String::new();
                    let mut names = Vec::new();
                    let mut is_wildcard = false;
                    let mut found_source = false;

                    for i in 0..import_node.child_count() {
                        if let Some(child) = import_node.child(i) {
                            match child.kind() {
                                "dotted_name" | "relative_import" => {
                                    if !found_source {
                                        source = child
                                            .utf8_text(&parsed.source[..])
                                            .unwrap_or("")
                                            .to_string();
                                        found_source = true;
                                    } else {
                                        // Second dotted_name is the imported name
                                        let name = child
                                            .utf8_text(&parsed.source[..])
                                            .unwrap_or("")
                                            .to_string();
                                        names.push(crate::extractors::ImportedName {
                                            local_name: name.clone(),
                                            original_name: Some(name),
                                        });
                                    }
                                }
                                "identifier" => {
                                    let name = child
                                        .utf8_text(&parsed.source[..])
                                        .unwrap_or("")
                                        .to_string();
                                    names.push(crate::extractors::ImportedName {
                                        local_name: name.clone(),
                                        original_name: Some(name),
                                    });
                                }
                                "wildcard_import" => {
                                    is_wildcard = true;
                                }
                                "aliased_import" => {
                                    let (orig, alias) = parse_py_aliased_import(child, parsed);
                                    names.push(crate::extractors::ImportedName {
                                        local_name: alias,
                                        original_name: Some(orig),
                                    });
                                }
                                _ => {}
                            }
                        }
                    }

                    imports.push(crate::extractors::ExtractedImport {
                        source,
                        names,
                        range,
                        is_wildcard,
                    });
                }
                _ => {}
            }
        }

        imports
    }
}

fn parse_py_aliased_import(node: Node, parsed: &ParsedFile) -> (String, String) {
    let mut orig = String::new();
    let mut alias = String::new();

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let text = child
                .utf8_text(&parsed.source[..])
                .unwrap_or("")
                .to_string();
            match child.kind() {
                "identifier" | "dotted_name" => {
                    if orig.is_empty() {
                        orig = text;
                    } else {
                        alias = text;
                    }
                }
                _ => {}
            }
        }
    }

    if alias.is_empty() {
        alias = orig.clone();
    }

    (orig, alias)
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct PythonHeritageExtractor;

impl crate::extractors::HeritageExtractor for PythonHeritageExtractor {
    fn language(&self) -> Language {
        Language::Python
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_py_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_py_heritage(
    parsed: &ParsedFile,
    node: Node,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    if node.kind() == "class_definition" {
        let child_id = node_id(parsed, &node);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i)
                && (child.kind() == "argument_list" || child.kind() == "base_classes")
            {
                for j in 0..child.child_count() {
                    if let Some(base) = child.child(j)
                        && matches!(base.kind(), "identifier" | "attribute")
                    {
                        let name = base.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
                        if name != "(" && name != ")" && name != "," && !name.is_empty() {
                            heritages.push(crate::extractors::ExtractedHeritage {
                                child_id,
                                parent_name: name,
                                kind: crate::extractors::HeritageKind::Extends,
                                range: base.range(),
                            });
                        }
                    }
                }
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_py_heritage(parsed, child, heritages);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::{CallExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_py(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.py");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.py"),
            size: code.len() as u64,
            language: Some(Language::Python),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_py_functions_and_classes() {
        let parsed = parse_py(
            r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

class Dog:
    def speak(self) -> str:
        return "Woof!"
"#,
        );
        let extractor = PythonSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "greet" && s.kind == NodeKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Dog" && s.kind == NodeKind::Class)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "speak" && s.kind == NodeKind::Function)
        );
    }

    #[test]
    fn extract_py_calls() {
        let parsed = parse_py(
            r#"
def foo():
    pass

class Bar:
    def baz(self):
        pass

def main():
    foo()
    b = Bar()
    b.baz()
    print("hello")
"#,
        );
        let extractor = PythonCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"foo"), "Expected foo call, got {:?}", names);
        assert!(names.contains(&"Bar"), "Expected Bar call, got {:?}", names);
        assert!(names.contains(&"baz"), "Expected baz call, got {:?}", names);
        assert!(
            names.contains(&"print"),
            "Expected print call, got {:?}",
            names
        );

        let member = calls.iter().find(|c| c.callee_name == "baz").unwrap();
        assert_eq!(member.receiver_name, Some("b".to_string()));
        assert_eq!(member.call_form, crate::extractors::CallForm::Member);
    }

    #[test]
    fn extract_py_imports() {
        let parsed = parse_py(
            r#"
import os
import os.path
from collections import defaultdict
from typing import List as L
from . import utils
"#,
        );
        let extractor = PythonImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "os" && i.names.iter().any(|n| n.local_name == "os")),
            "Expected import os"
        );
        assert!(
            imports.iter().any(|i| i.source == "os.path"),
            "Expected import os.path"
        );
        assert!(
            imports.iter().any(|i| i.source == "collections"
                && i.names.iter().any(|n| n.local_name == "defaultdict")),
            "Expected from collections import defaultdict"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "typing" && i.names.iter().any(|n| n.local_name == "L")),
            "Expected from typing import List as L"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "." && i.names.iter().any(|n| n.local_name == "utils")),
            "Expected from . import utils"
        );
    }
}

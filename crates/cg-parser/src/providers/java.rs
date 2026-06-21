//! Java language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{
    CallExtractor, ExtractedCall, ExtractedHeritage, ExtractedImport, ExtractedSymbol,
    HeritageExtractor, HeritageKind, ImportExtractor, ImportedName, SymbolExtractor,
};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

const JAVA_QUERIES: &str = r#"
;; Definitions
(class_declaration name: (identifier) @definition.class) @def.class
(interface_declaration name: (identifier) @definition.interface) @def.interface
(method_declaration name: (identifier) @definition.method) @def.method
(constructor_declaration name: (identifier) @definition.constructor) @def.constructor
(enum_declaration name: (identifier) @definition.enum) @def.enum
"#;

pub struct JavaProvider;

impl LanguageProvider for JavaProvider {
    fn id(&self) -> Language {
        Language::Java
    }

    fn extensions(&self) -> &[&str] {
        &["java"]
    }

    fn tree_sitter_queries(&self) -> &str {
        JAVA_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::Named
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::FirstWins
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(JavaSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn CallExtractor>> {
        Some(Box::new(JavaCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn ImportExtractor>> {
        Some(Box::new(JavaImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn HeritageExtractor>> {
        Some(Box::new(JavaHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        _from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        let target_path = target.replace('.', "/");
        let candidate = PathBuf::from(&target_path).with_extension("java");
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
        None
    }

    fn is_exported(&self, _name: &str, node: &Node, source: &[u8]) -> bool {
        has_public_modifier(*node, source)
            || ancestor_with_kind(
                *node,
                &[
                    "class_declaration",
                    "interface_declaration",
                    "enum_declaration",
                ],
            )
            .is_some_and(|n| has_public_modifier(n, source))
    }
}

/// Check whether a node (or its `modifiers` child) contains the `public` keyword.
fn has_public_modifier(node: Node, _source: &[u8]) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == "modifiers"
        {
            for j in 0..child.child_count() {
                if let Some(m) = child.child(j)
                    && m.kind() == "public"
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk up the tree to find an ancestor with one of the given kinds.
fn ancestor_with_kind<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if kinds.iter().any(|&k| parent.kind() == k) {
            return Some(parent);
        }
        current = parent;
    }
    None
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
// Symbol Extractor
// ============================================================================

struct JavaSymbolExtractor;

impl SymbolExtractor for JavaSymbolExtractor {
    fn language(&self) -> Language {
        Language::Java
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let query = match build_query(grammar, JAVA_QUERIES) {
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
                "definition.class" => (
                    NodeKind::Class,
                    parent_kind(node, &["class_declaration"]).unwrap_or(node),
                ),
                "definition.interface" => (
                    NodeKind::Interface,
                    parent_kind(node, &["interface_declaration"]).unwrap_or(node),
                ),
                "definition.method" => (
                    NodeKind::Function,
                    parent_kind(node, &["method_declaration"]).unwrap_or(node),
                ),
                "definition.constructor" => (
                    NodeKind::Function,
                    parent_kind(node, &["constructor_declaration"]).unwrap_or(node),
                ),
                "definition.enum" => (
                    NodeKind::Class,
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

// ============================================================================
// Call Extractor
// ============================================================================

struct JavaCallExtractor;

impl CallExtractor for JavaCallExtractor {
    fn language(&self) -> Language {
        Language::Java
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedCall> {
        let mut calls = Vec::new();
        extract_java_calls(parsed, parsed.tree.root_node(), &mut calls);
        calls
    }
}

fn extract_java_calls(parsed: &ParsedFile, node: Node, calls: &mut Vec<ExtractedCall>) {
    match node.kind() {
        "method_invocation" => {
            if let Some(callee) = extract_method_invocation_callee(node, parsed) {
                let caller_id = find_enclosing_symbol_id_java(parsed, node);
                let range = node.range();
                let arg_count = count_arguments_java(node);
                calls.push(ExtractedCall {
                    caller_id,
                    callee_name: callee.name,
                    call_form: callee.form,
                    range,
                    receiver_name: callee.receiver,
                    argument_count: arg_count,
                });
            }
        }
        "object_creation_expression" => {
            if let Some(type_node) = find_child_by_kind(node, "type_identifier") {
                let caller_id = find_enclosing_symbol_id_java(parsed, node);
                let range = node.range();
                let arg_count = count_arguments_java(node);
                let name = type_node
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string();
                calls.push(ExtractedCall {
                    caller_id,
                    callee_name: name,
                    call_form: crate::extractors::CallForm::Constructor,
                    range,
                    receiver_name: None,
                    argument_count: arg_count,
                });
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_java_calls(parsed, child, calls);
        }
    }
}

struct CalleeInfo {
    name: String,
    form: crate::extractors::CallForm,
    receiver: Option<String>,
}

fn extract_method_invocation_callee(node: Node, parsed: &ParsedFile) -> Option<CalleeInfo> {
    // method_invocation children: receiver?, ., identifier, argument_list
    let mut receiver: Option<String> = None;
    let mut method_name: Option<String> = None;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "identifier" | "field_access" | "method_invocation" => {
                    // If we haven't seen a dot yet, this is the receiver.
                    // If we have, this is the method name.
                    let text = child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                    if method_name.is_none() && receiver.is_none() {
                        // Could be receiver or method (if no dot)
                        receiver = Some(text);
                    } else if method_name.is_none() {
                        method_name = Some(text);
                    }
                }
                "." => {
                    // The preceding identifier becomes the receiver
                    if let Some(prev) = receiver.take() {
                        receiver = Some(prev);
                    }
                }
                "argument_list" => {
                    // We've passed the method name by now
                }
                _ => {}
            }
        }
    }

    // Re-evaluate: in Java method_invocation, the structure is typically:
    //   identifier "." identifier argument_list
    // or just identifier argument_list (for unqualified calls)
    // Let's do a simpler approach based on child positions.

    let mut children: Vec<Node> = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            children.push(child);
        }
    }

    // Find the argument_list position
    let arg_list_idx = children.iter().position(|c| c.kind() == "argument_list");

    if let Some(idx) = arg_list_idx {
        // The method name is the identifier just before argument_list (skipping dots)
        let mut method_idx = idx.checked_sub(1)?;
        while method_idx > 0 && children[method_idx].kind() == "." {
            method_idx = method_idx.checked_sub(1)?;
        }
        if children[method_idx].kind() == "identifier" {
            method_name = Some(
                children[method_idx]
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string(),
            );
        }

        // Everything before the method name (and its dot) is the receiver
        if method_idx > 1 {
            // There is a receiver chain
            let receiver_nodes = &children[..method_idx - 1]; // exclude the dot before method
            if !receiver_nodes.is_empty() {
                let start = receiver_nodes.first()?.start_byte();
                let end = receiver_nodes.last()?.end_byte();
                let text = String::from_utf8_lossy(&parsed.source[start..end]).to_string();
                receiver = Some(text);
            }
        } else if method_idx == 1 && children[0].kind() == "identifier" {
            // Single identifier receiver with dot: obj.method()
            receiver = Some(
                children[0]
                    .utf8_text(&parsed.source[..])
                    .unwrap_or("")
                    .to_string(),
            );
        }
    }

    let name = method_name?;
    let form = if receiver.is_some() {
        crate::extractors::CallForm::Member
    } else {
        crate::extractors::CallForm::Free
    };

    Some(CalleeInfo {
        name,
        form,
        receiver,
    })
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == kind
        {
            return Some(child);
        }
    }
    None
}

fn find_enclosing_symbol_id_java(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "method_declaration"
            | "constructor_declaration"
            | "lambda_expression"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments_java(node: Node) -> usize {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
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

struct JavaImportExtractor;

impl ImportExtractor for JavaImportExtractor {
    fn language(&self) -> Language {
        Language::Java
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedImport> {
        let mut imports = Vec::new();
        extract_java_imports(parsed, parsed.tree.root_node(), &mut imports);
        imports
    }
}

fn extract_java_imports(parsed: &ParsedFile, node: Node, imports: &mut Vec<ExtractedImport>) {
    if node.kind() == "import_declaration" {
        let range = node.range();
        let mut source = String::new();
        let mut is_wildcard = false;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "scoped_identifier" | "identifier" => {
                        source = child
                            .utf8_text(&parsed.source[..])
                            .unwrap_or("")
                            .to_string();
                    }
                    "asterisk" => {
                        is_wildcard = true;
                    }
                    _ => {
                        // Some grammars represent wildcard as a standalone "." + "*"
                        let text = child
                            .utf8_text(&parsed.source[..])
                            .unwrap_or("")
                            .to_string();
                        if text == "*" {
                            is_wildcard = true;
                        }
                    }
                }
            }
        }

        let mut names = Vec::new();
        if !is_wildcard && !source.is_empty() {
            let name = source.split('.').next_back().unwrap_or(&source).to_string();
            names.push(ImportedName {
                local_name: name,
                original_name: Some(source.clone()),
            });
        }

        imports.push(ExtractedImport {
            source,
            names,
            range,
            is_wildcard,
        });
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_java_imports(parsed, child, imports);
        }
    }
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct JavaHeritageExtractor;

impl HeritageExtractor for JavaHeritageExtractor {
    fn language(&self) -> Language {
        Language::Java
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_java_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_java_heritage(parsed: &ParsedFile, node: Node, heritages: &mut Vec<ExtractedHeritage>) {
    if node.kind() == "class_declaration" || node.kind() == "interface_declaration" {
        let child_id = node_id(parsed, &node);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "superclass" => {
                        if let Some(type_id) = find_child_by_kind(child, "type_identifier") {
                            let name = type_id
                                .utf8_text(&parsed.source[..])
                                .unwrap_or("")
                                .to_string();
                            if !name.is_empty() {
                                heritages.push(ExtractedHeritage {
                                    child_id,
                                    parent_name: name,
                                    kind: HeritageKind::Extends,
                                    range: type_id.range(),
                                });
                            }
                        }
                    }
                    "super_interfaces" => {
                        extract_type_list(
                            parsed,
                            child,
                            child_id,
                            heritages,
                            HeritageKind::Implements,
                        );
                    }
                    "extends_interfaces" => {
                        extract_type_list(
                            parsed,
                            child,
                            child_id,
                            heritages,
                            HeritageKind::Extends,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_java_heritage(parsed, child, heritages);
        }
    }
}

fn extract_type_list(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    heritages: &mut Vec<ExtractedHeritage>,
    kind: HeritageKind,
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
                        kind,
                        range: child.range(),
                    });
                }
            } else {
                extract_type_list(parsed, child, child_id, heritages, kind);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_java(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Test.java");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("Test.java"),
            size: code.len() as u64,
            language: Some(Language::Java),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_java_symbols() {
        let parsed = parse_java(
            r#"
import com.example.Foo;

public class Bar extends Baz implements Qux, Quux {
    public void hello() {
        Foo.f();
        new Object();
    }
    public Bar() {}
}

interface MyInterface {}
enum MyEnum { A, B }
"#,
        );
        let extractor = JavaSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Bar" && s.kind == NodeKind::Class),
            "Expected class Bar"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "hello" && s.kind == NodeKind::Function),
            "Expected method hello"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Bar" && s.kind == NodeKind::Function),
            "Expected constructor Bar"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyInterface" && s.kind == NodeKind::Interface),
            "Expected interface MyInterface"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyEnum" && s.kind == NodeKind::Class),
            "Expected enum MyEnum"
        );
    }

    #[test]
    fn extract_java_calls() {
        let parsed = parse_java(
            r#"
public class Test {
    public void main() {
        foo();
        obj.bar();
        new Object();
    }
}
"#,
        );
        let extractor = JavaCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"foo"), "Expected foo call, got {:?}", names);
        assert!(names.contains(&"bar"), "Expected bar call, got {:?}", names);
        assert!(
            names.contains(&"Object"),
            "Expected Object constructor, got {:?}",
            names
        );

        let member = calls.iter().find(|c| c.callee_name == "bar").unwrap();
        assert_eq!(member.receiver_name, Some("obj".to_string()));
        assert_eq!(member.call_form, crate::extractors::CallForm::Member);

        let ctor = calls.iter().find(|c| c.callee_name == "Object").unwrap();
        assert_eq!(ctor.call_form, crate::extractors::CallForm::Constructor);
    }

    #[test]
    fn extract_java_imports() {
        let parsed = parse_java(
            r#"
import com.example.Foo;
import java.util.*;
import static org.junit.Assert.assertTrue;
"#,
        );
        let extractor = JavaImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "com.example.Foo" && !i.is_wildcard),
            "Expected import com.example.Foo"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "java.util" && i.is_wildcard),
            "Expected wildcard import java.util.*"
        );
    }

    #[test]
    fn extract_java_heritage() {
        let parsed = parse_java(
            r#"
public class Dog extends Animal implements Named, Walkable {}
interface Special extends Named {}
"#,
        );
        let extractor = JavaHeritageExtractor;
        let heritages = extractor.extract(&parsed);
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Animal" && h.kind == HeritageKind::Extends),
            "Expected extends Animal"
        );
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Named" && h.kind == HeritageKind::Implements),
            "Expected implements Named"
        );
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Walkable" && h.kind == HeritageKind::Implements),
            "Expected implements Walkable"
        );
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Named" && h.kind == HeritageKind::Extends),
            "Expected interface extends Named"
        );
    }

    #[test]
    fn java_provider_id_and_extensions() {
        let provider = JavaProvider;
        assert_eq!(provider.id(), Language::Java);
        assert_eq!(provider.extensions(), &["java"]);
        assert_eq!(provider.import_semantics(), ImportSemantics::Named);
        assert_eq!(provider.mro_strategy(), MroStrategy::FirstWins);
    }

    #[test]
    fn java_resolve_import() {
        let provider = JavaProvider;
        let mut files = HashSet::new();
        files.insert(PathBuf::from("com/example/Foo.java"));
        let resolved =
            provider.resolve_import("com.example.Foo", Path::new("src/Main.java"), &files);
        assert_eq!(resolved, Some(PathBuf::from("com/example/Foo.java")));
    }

    #[test]
    fn java_is_exported_public_class() {
        let parsed = parse_java("public class Foo { public void bar() {} }");
        let provider = JavaProvider;
        let class_node = parsed.tree.root_node();
        // Find the method_declaration node
        let mut method_node = None;
        for i in 0..class_node.child_count() {
            if let Some(child) = class_node.child(i)
                && child.kind() == "class_declaration"
            {
                for j in 0..child.child_count() {
                    if let Some(c) = child.child(j)
                        && c.kind() == "class_body"
                    {
                        for k in 0..c.child_count() {
                            if let Some(m) = c.child(k)
                                && m.kind() == "method_declaration"
                            {
                                method_node = Some(m);
                                break;
                            }
                        }
                    }
                }
            }
        }
        let method = method_node.unwrap();
        assert!(provider.is_exported("bar", &method, &parsed.source));
    }
}

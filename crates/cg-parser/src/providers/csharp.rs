//! C# language provider.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy, NodeId, NodeKind};
use tree_sitter::{Node, QueryCursor, StreamingIterator};

use crate::extractors::{ExtractedSymbol, SymbolExtractor};
use crate::parser::ParsedFile;
use crate::providers::{LanguageProvider, build_query};

const CSHARP_QUERIES: &str = r#"
;; Definitions
(class_declaration name: (identifier) @definition.class) @def.class
(interface_declaration name: (identifier) @definition.interface) @def.interface
(struct_declaration name: (identifier) @definition.struct) @def.struct
(enum_declaration name: (identifier) @definition.enum) @def.enum
(method_declaration name: (identifier) @definition.method) @def.method
(namespace_declaration name: (_) @definition.namespace) @def.namespace
(property_declaration name: (identifier) @definition.property) @def.property
"#;

pub struct CSharpProvider;

impl LanguageProvider for CSharpProvider {
    fn id(&self) -> Language {
        Language::CSharp
    }

    fn extensions(&self) -> &[&str] {
        &["cs"]
    }

    fn tree_sitter_queries(&self) -> &str {
        CSHARP_QUERIES
    }

    fn import_semantics(&self) -> ImportSemantics {
        ImportSemantics::Namespace
    }

    fn mro_strategy(&self) -> MroStrategy {
        MroStrategy::FirstWins
    }

    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>> {
        Some(Box::new(CSharpSymbolExtractor))
    }

    fn call_extractor(&self) -> Option<Box<dyn crate::extractors::CallExtractor>> {
        Some(Box::new(CSharpCallExtractor))
    }

    fn import_extractor(&self) -> Option<Box<dyn crate::extractors::ImportExtractor>> {
        Some(Box::new(CSharpImportExtractor))
    }

    fn heritage_extractor(&self) -> Option<Box<dyn crate::extractors::HeritageExtractor>> {
        Some(Box::new(CSharpHeritageExtractor))
    }

    fn resolve_import(
        &self,
        target: &str,
        _from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        let target_path = target.replace('.', "/");
        let candidate = PathBuf::from(&target_path).with_extension("cs");
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
        None
    }

    fn is_exported(&self, _name: &str, node: &Node, source: &[u8]) -> bool {
        has_public_modifier_cs(*node, source)
            || ancestor_with_kind(
                *node,
                &[
                    "class_declaration",
                    "interface_declaration",
                    "struct_declaration",
                    "enum_declaration",
                    "method_declaration",
                    "property_declaration",
                    "namespace_declaration",
                ],
            )
            .is_some_and(|n| has_public_modifier_cs(n, source))
    }
}

// ============================================================================
// Symbol Extractor
// ============================================================================

struct CSharpSymbolExtractor;

impl SymbolExtractor for CSharpSymbolExtractor {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<ExtractedSymbol> {
        let mut symbols = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
        let query = match build_query(grammar, CSHARP_QUERIES) {
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
                "definition.struct" => (
                    NodeKind::Struct,
                    parent_kind(node, &["struct_declaration"]).unwrap_or(node),
                ),
                "definition.enum" => (
                    NodeKind::Enum,
                    parent_kind(node, &["enum_declaration"]).unwrap_or(node),
                ),
                "definition.method" => (
                    NodeKind::Method,
                    parent_kind(node, &["method_declaration"]).unwrap_or(node),
                ),
                "definition.namespace" => (
                    NodeKind::Namespace,
                    parent_kind(node, &["namespace_declaration"]).unwrap_or(node),
                ),
                "definition.property" => (
                    NodeKind::Property,
                    parent_kind(node, &["property_declaration"]).unwrap_or(node),
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

fn has_public_modifier_cs(node: Node, source: &[u8]) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == "modifier"
        {
            let text = child.utf8_text(source).unwrap_or("");
            if text == "public" {
                return true;
            }
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

struct CSharpCallExtractor;

impl crate::extractors::CallExtractor for CSharpCallExtractor {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedCall> {
        let mut calls = Vec::new();
        let grammar: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();

        let query_text = r#"
        (invocation_expression
          function: (identifier) @call.function) @call

        (invocation_expression
          function: (member_access_expression
            expression: (_) @call.object
            name: (identifier) @call.function)) @call

        (object_creation_expression
          type: (identifier) @call.function) @call
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
            let caller_id = find_enclosing_symbol_id_csharp(parsed, call);
            let range = call.range();
            let arg_count = count_arguments_csharp(call);

            let callee_name = func.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
            let is_constructor = call.kind() == "object_creation_expression";

            let (call_form, receiver_name) = if is_constructor {
                (crate::extractors::CallForm::Constructor, None)
            } else if let Some(obj) = object_node {
                let receiver = obj.utf8_text(&parsed.source[..]).unwrap_or("").to_string();
                (crate::extractors::CallForm::Member, Some(receiver))
            } else {
                (crate::extractors::CallForm::Free, None)
            };

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

fn find_enclosing_symbol_id_csharp(parsed: &ParsedFile, node: Node) -> NodeId {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "method_declaration" | "constructor_declaration" | "destructor_declaration" => {
                return node_id(parsed, &parent);
            }
            "class_declaration"
            | "interface_declaration"
            | "struct_declaration"
            | "enum_declaration" => {
                return node_id(parsed, &parent);
            }
            _ => current = parent,
        }
    }
    node_id(parsed, &node)
}

fn count_arguments_csharp(call_node: Node) -> usize {
    for i in 0..call_node.child_count() {
        if let Some(child) = call_node.child(i)
            && child.kind() == "argument_list"
        {
            let mut count = 0;
            for j in 0..child.child_count() {
                if let Some(arg) = child.child(j)
                    && arg.kind() == "argument"
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

struct CSharpImportExtractor;

impl crate::extractors::ImportExtractor for CSharpImportExtractor {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedImport> {
        let mut imports = Vec::new();
        extract_csharp_imports(parsed, parsed.tree.root_node(), &mut imports);
        imports
    }
}

fn extract_csharp_imports(
    parsed: &ParsedFile,
    node: Node,
    imports: &mut Vec<crate::extractors::ExtractedImport>,
) {
    if node.kind() == "using_directive" {
        let range = node.range();
        let mut source = String::new();
        let mut names = Vec::new();
        let mut is_wildcard = false;
        let mut has_alias = false;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                match child.kind() {
                    "identifier" | "qualified_name" => {
                        let text = child
                            .utf8_text(&parsed.source[..])
                            .unwrap_or("")
                            .to_string();
                        if source.is_empty() {
                            source = text;
                        } else if has_alias {
                            // Second identifier after '=' is the alias name
                            names.push(crate::extractors::ImportedName {
                                local_name: source.clone(),
                                original_name: Some(text),
                            });
                        }
                    }
                    "=" => {
                        has_alias = true;
                        // source currently holds the alias; we need to swap later
                    }
                    _ => {}
                }
            }
        }

        if has_alias && !names.is_empty() {
            // Aliased import: using Alias = System.Text;
            // names already populated above
        } else if !source.is_empty() {
            // Namespace import: using System.Collections.Generic;
            is_wildcard = true;
        }

        imports.push(crate::extractors::ExtractedImport {
            source,
            names,
            range,
            is_wildcard,
        });
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_csharp_imports(parsed, child, imports);
        }
    }
}

// ============================================================================
// Heritage Extractor
// ============================================================================

struct CSharpHeritageExtractor;

impl crate::extractors::HeritageExtractor for CSharpHeritageExtractor {
    fn language(&self) -> Language {
        Language::CSharp
    }

    fn extract(&self, parsed: &ParsedFile) -> Vec<crate::extractors::ExtractedHeritage> {
        let mut heritages = Vec::new();
        extract_csharp_heritage(parsed, parsed.tree.root_node(), &mut heritages);
        heritages
    }
}

fn extract_csharp_heritage(
    parsed: &ParsedFile,
    node: Node,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    match node.kind() {
        "class_declaration" | "interface_declaration" | "struct_declaration" => {
            let child_id = node_id(parsed, &node);
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i)
                    && child.kind() == "base_list"
                {
                    extract_base_list(parsed, child, child_id, node.kind(), heritages);
                }
            }
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_csharp_heritage(parsed, child, heritages);
        }
    }
}

fn extract_base_list(
    parsed: &ParsedFile,
    node: Node,
    child_id: NodeId,
    parent_kind: &str,
    heritages: &mut Vec<crate::extractors::ExtractedHeritage>,
) {
    let mut idx = 0;
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "identifier" | "qualified_name" => {
                    let name = child
                        .utf8_text(&parsed.source[..])
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() || name == ":" || name == "," {
                        continue;
                    }
                    let kind = match parent_kind {
                        "class_declaration" => {
                            if idx == 0 {
                                crate::extractors::HeritageKind::Extends
                            } else {
                                crate::extractors::HeritageKind::Implements
                            }
                        }
                        "interface_declaration" => crate::extractors::HeritageKind::Extends,
                        "struct_declaration" => crate::extractors::HeritageKind::Implements,
                        _ => crate::extractors::HeritageKind::Extends,
                    };
                    heritages.push(crate::extractors::ExtractedHeritage {
                        child_id,
                        parent_name: name,
                        kind,
                        range: child.range(),
                    });
                    idx += 1;
                }
                _ => {}
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
    use crate::extractors::{CallExtractor, HeritageExtractor, ImportExtractor};
    use crate::parser::pool::ParserPool;
    use crate::scanner::FileInfo;

    fn parse_cs(code: &str) -> ParsedFile {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.cs");
        std::fs::write(&path, code).unwrap();
        let file = FileInfo {
            path: path.clone(),
            relative_path: std::path::PathBuf::from("test.cs"),
            size: code.len() as u64,
            language: Some(Language::CSharp),
        };
        let pool = ParserPool::new(1).unwrap();
        pool.parse_file(&file).unwrap()
    }

    #[test]
    fn extract_csharp_symbols() {
        let parsed = parse_cs(
            r#"
namespace MyApp.Models
{
    public class User
    {
        public string Name { get; set; }
        public void Greet() { }
    }

    public interface IUser { }
    public struct Point { }
    public enum Status { Active, Inactive }
}
"#,
        );
        let extractor = CSharpSymbolExtractor;
        let symbols = extractor.extract(&parsed);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "MyApp.Models" && s.kind == NodeKind::Namespace),
            "Expected namespace MyApp.Models"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "User" && s.kind == NodeKind::Class),
            "Expected class User"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "IUser" && s.kind == NodeKind::Interface),
            "Expected interface IUser"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Point" && s.kind == NodeKind::Struct),
            "Expected struct Point"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Status" && s.kind == NodeKind::Enum),
            "Expected enum Status"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Greet" && s.kind == NodeKind::Method),
            "Expected method Greet"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Name" && s.kind == NodeKind::Property),
            "Expected property Name"
        );
    }

    #[test]
    fn extract_csharp_calls() {
        let parsed = parse_cs(
            r#"
class Program
{
    static void Main()
    {
        Foo();
        var x = new Bar();
        x.Baz();
        Console.WriteLine("hi");
    }
}
"#,
        );
        let extractor = CSharpCallExtractor;
        let calls = extractor.extract(&parsed);
        let names: Vec<_> = calls.iter().map(|c| c.callee_name.as_str()).collect();
        assert!(names.contains(&"Foo"), "Expected Foo call, got {:?}", names);
        assert!(
            names.contains(&"Bar"),
            "Expected Bar constructor, got {:?}",
            names
        );
        assert!(names.contains(&"Baz"), "Expected Baz call, got {:?}", names);
        assert!(
            names.contains(&"WriteLine"),
            "Expected WriteLine call, got {:?}",
            names
        );

        let member = calls.iter().find(|c| c.callee_name == "Baz").unwrap();
        assert_eq!(member.receiver_name, Some("x".to_string()));
        assert_eq!(member.call_form, crate::extractors::CallForm::Member);

        let constructor = calls.iter().find(|c| c.callee_name == "Bar").unwrap();
        assert_eq!(
            constructor.call_form,
            crate::extractors::CallForm::Constructor
        );
    }

    #[test]
    fn extract_csharp_imports() {
        let parsed = parse_cs(
            r#"
using System;
using System.Collections.Generic;
using Alias = System.Text;
"#,
        );
        let extractor = CSharpImportExtractor;
        let imports = extractor.extract(&parsed);
        assert!(
            imports
                .iter()
                .any(|i| i.source == "System" && i.is_wildcard),
            "Expected using System"
        );
        assert!(
            imports
                .iter()
                .any(|i| i.source == "System.Collections.Generic" && i.is_wildcard),
            "Expected using System.Collections.Generic"
        );
        assert!(
            imports.iter().any(|i| i.source == "Alias"
                && i.names.iter().any(|n| n.local_name == "Alias"
                    && n.original_name.as_deref() == Some("System.Text"))
                && !i.is_wildcard),
            "Expected using Alias = System.Text"
        );
    }

    #[test]
    fn extract_csharp_heritage() {
        let parsed = parse_cs(
            r#"
public class Animal { }
public interface Named { }
public interface Movable { }
public class Dog : Animal, Named, Movable { }
public interface IFancy : Named, Movable { }
public struct Point : Movable { }
"#,
        );
        let extractor = CSharpHeritageExtractor;
        let heritages = extractor.extract(&parsed);
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Animal"
                    && h.kind == crate::extractors::HeritageKind::Extends),
            "Expected extends Animal"
        );
        assert!(
            heritages.iter().any(|h| h.parent_name == "Named"
                && h.kind == crate::extractors::HeritageKind::Implements),
            "Expected implements Named"
        );
        assert!(
            heritages.iter().any(|h| h.parent_name == "Movable"
                && h.kind == crate::extractors::HeritageKind::Implements),
            "Expected implements Movable"
        );
        assert!(
            heritages
                .iter()
                .any(|h| h.parent_name == "Named"
                    && h.kind == crate::extractors::HeritageKind::Extends),
            "Expected interface extends Named"
        );
        assert!(
            heritages.iter().any(|h| h.parent_name == "Movable"
                && h.kind == crate::extractors::HeritageKind::Extends),
            "Expected interface extends Movable"
        );
        assert!(
            heritages.iter().any(|h| h.parent_name == "Movable"
                && h.kind == crate::extractors::HeritageKind::Implements),
            "Expected struct implements Movable"
        );
    }

    #[test]
    fn csharp_is_exported() {
        let provider = CSharpProvider;
        let parsed = parse_cs(
            r#"
public class PublicClass { }
class InternalClass { }
"#,
        );
        let root = parsed.tree.root_node();
        // Find the class_declaration nodes
        let mut public_class_node = None;
        let mut internal_class_node = None;
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i)
                && child.kind() == "class_declaration"
            {
                let text = child.utf8_text(&parsed.source[..]).unwrap_or("");
                if text.contains("PublicClass") {
                    public_class_node = Some(child);
                } else if text.contains("InternalClass") {
                    internal_class_node = Some(child);
                }
            }
        }
        assert!(public_class_node.is_some());
        assert!(internal_class_node.is_some());
        assert!(provider.is_exported(
            "PublicClass",
            &public_class_node.unwrap(),
            &parsed.source[..]
        ));
        assert!(!provider.is_exported(
            "InternalClass",
            &internal_class_node.unwrap(),
            &parsed.source[..]
        ));
    }

    #[test]
    fn csharp_resolve_import() {
        let provider = CSharpProvider;
        let mut files = HashSet::new();
        files.insert(PathBuf::from("System/Collections/Generic.cs"));
        let resolved =
            provider.resolve_import("System.Collections.Generic", Path::new("test.cs"), &files);
        assert_eq!(
            resolved,
            Some(PathBuf::from("System/Collections/Generic.cs"))
        );
    }
}

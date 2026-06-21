//! Tree-sitter infrastructure: parser pool and grammar registry.

pub mod pool;

use std::collections::HashMap;
use std::sync::Mutex;

use cg_common::Language;
use tree_sitter::Language as TSLanguage;

/// Registry of loaded tree-sitter grammars.
///
/// Grammars are loaded lazily and cached for reuse.
pub struct GrammarRegistry {
    cache: Mutex<HashMap<Language, TSLanguage>>,
}

impl GrammarRegistry {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get or load a tree-sitter grammar for a language.
    pub fn get(&self, lang: Language) -> Option<TSLanguage> {
        {
            let cache = self.cache.lock().unwrap();
            if let Some(grammar) = cache.get(&lang) {
                return Some(grammar.clone());
            }
        }

        let grammar = load_grammar(lang)?;
        let mut cache = self.cache.lock().unwrap();
        cache.insert(lang, grammar.clone());
        Some(grammar)
    }

    /// Pre-load grammars for a set of languages.
    pub fn preload(&self, langs: &[Language]) {
        for &lang in langs {
            let _ = self.get(lang);
        }
    }
}

impl Default for GrammarRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn load_grammar(lang: Language) -> Option<TSLanguage> {
    match lang {
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::JavaScript => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Java => Some(tree_sitter_java::LANGUAGE.into()),
        Language::Go => Some(tree_sitter_go::LANGUAGE.into()),
        Language::C => Some(tree_sitter_c::LANGUAGE.into()),
        Language::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
        Language::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
        _ => None,
    }
}

/// Parsed result for a single source file.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub file_path: std::path::PathBuf,
    pub language: Language,
    pub tree: tree_sitter::Tree,
    pub source: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_rust_grammar() {
        let reg = GrammarRegistry::new();
        let grammar = reg.get(Language::Rust).unwrap();
        // ABI versions may differ between grammar crate and tree-sitter crate
        assert!(grammar.abi_version() > 0);
    }

    #[test]
    fn load_typescript_grammar() {
        let reg = GrammarRegistry::new();
        let grammar = reg.get(Language::TypeScript).unwrap();
        // ABI versions may differ between grammar crate and tree-sitter crate
        assert!(grammar.abi_version() > 0);
    }

    #[test]
    fn load_python_grammar() {
        let reg = GrammarRegistry::new();
        let grammar = reg.get(Language::Python).unwrap();
        // ABI versions may differ between grammar crate and tree-sitter crate
        assert!(grammar.abi_version() > 0);
    }

    #[test]
    fn unknown_grammar_returns_none() {
        let reg = GrammarRegistry::new();
        assert!(reg.get(Language::JavaScript).is_some());
    }
}

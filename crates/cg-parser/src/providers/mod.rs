//! Language providers and registry.

pub mod c;
pub mod cpp;
pub mod csharp;
pub mod go;
pub mod java;
pub mod python;
pub mod rust;
pub mod typescript;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy};

use crate::extractors::{CallExtractor, HeritageExtractor, ImportExtractor, SymbolExtractor};

/// Unified language provider trait.
pub trait LanguageProvider: Send + Sync {
    /// Unique language identifier.
    fn id(&self) -> Language;

    /// File extensions associated with this language.
    fn extensions(&self) -> &[&str];

    /// Tree-sitter S-expression queries for symbol extraction.
    fn tree_sitter_queries(&self) -> &str;

    /// How this language resolves imported names.
    fn import_semantics(&self) -> ImportSemantics;

    /// MRO strategy for this language.
    fn mro_strategy(&self) -> MroStrategy;

    /// Symbol extractor for this language.
    fn symbol_extractor(&self) -> Option<Box<dyn SymbolExtractor>>;

    /// Call extractor for this language.
    fn call_extractor(&self) -> Option<Box<dyn CallExtractor>>;

    /// Import extractor for this language.
    fn import_extractor(&self) -> Option<Box<dyn ImportExtractor>>;

    /// Heritage extractor for this language.
    fn heritage_extractor(&self) -> Option<Box<dyn HeritageExtractor>>;

    /// Resolve an import target string to an absolute file path.
    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &std::collections::HashSet<PathBuf>,
    ) -> Option<PathBuf>;

    /// Determine whether a symbol is exported.
    fn is_exported(&self, name: &str, node: &tree_sitter::Node, source: &[u8]) -> bool;
}

/// Registry of all supported language providers.
pub struct ProviderRegistry {
    providers: HashMap<Language, Box<dyn LanguageProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            providers: HashMap::new(),
        };
        // Register built-in providers
        registry.register(Box::new(c::CProvider));
        registry.register(Box::new(cpp::CppProvider));
        registry.register(Box::new(csharp::CSharpProvider));
        registry.register(Box::new(go::GoProvider));
        registry.register(Box::new(java::JavaProvider));
        registry.register(Box::new(python::PythonProvider));
        registry.register(Box::new(rust::RustProvider));
        registry.register(Box::new(typescript::TypeScriptProvider));
        registry
    }

    pub fn register(&mut self, provider: Box<dyn LanguageProvider>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn get(&self, lang: Language) -> Option<&dyn LanguageProvider> {
        self.providers.get(&lang).map(|p| p.as_ref())
    }

    pub fn get_by_extension(&self, ext: &str) -> Option<&dyn LanguageProvider> {
        self.providers
            .values()
            .find(|p| p.extensions().contains(&ext))
            .map(|p| p.as_ref())
    }

    pub fn all(&self) -> Vec<Language> {
        self.providers.keys().copied().collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper: build a tree-sitter query from a provider's queries string.
pub fn build_query(
    lang: tree_sitter::Language,
    query_text: &str,
) -> anyhow::Result<tree_sitter::Query> {
    tree_sitter::Query::new(&lang, query_text)
        .map_err(|e| anyhow::anyhow!("Query error at {}: {:?}", e.offset, e))
}

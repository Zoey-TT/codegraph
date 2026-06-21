//! Language providers and trait definitions.
//!
//! Each supported language implements `LanguageProvider` to supply:
//! - File extensions
//! - Tree-sitter queries
//! - Import semantics and resolution logic

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::{ImportSemantics, Language, MroStrategy};

/// Trait for language-specific parsing configuration.
pub trait LanguageProvider: Send + Sync {
    /// Unique language identifier.
    fn id(&self) -> Language;

    /// File extensions associated with this language.
    fn extensions(&self) -> &[&str];

    /// Tree-sitter S-expression queries for symbol extraction.
    fn tree_sitter_queries(&self) -> &str;

    /// How this language resolves imported names.
    fn import_semantics(&self) -> ImportSemantics;

    /// Resolve an import target string to an absolute file path.
    fn resolve_import(
        &self,
        target: &str,
        from: &Path,
        all_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf>;

    /// Determine whether a symbol is exported.
    fn is_exported(&self, name: &str, node: &str, source: &[u8]) -> bool;

    /// MRO strategy for this language.
    fn mro_strategy(&self) -> MroStrategy;
}

/// Registry of all supported language providers.
pub struct LanguageRegistry {
    providers: Vec<Box<dyn LanguageProvider>>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn LanguageProvider>) {
        self.providers.push(provider);
    }

    pub fn get(&self, lang: Language) -> Option<&dyn LanguageProvider> {
        self.providers
            .iter()
            .find(|p| p.id() == lang)
            .map(|p| p.as_ref())
    }

    pub fn get_by_extension(&self, ext: &str) -> Option<&dyn LanguageProvider> {
        self.providers
            .iter()
            .find(|p| p.extensions().contains(&ext))
            .map(|p| p.as_ref())
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

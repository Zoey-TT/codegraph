//! Parser pool for parallel tree-sitter parsing.
//!
//! Each thread in the pool maintains its own `Parser` instances
//! (tree-sitter `Parser` is not thread-safe).

use std::cell::RefCell;
use std::sync::Arc;

use rayon::prelude::*;
use tree_sitter::Parser;

use cg_common::Language;

use crate::parser::{GrammarRegistry, ParsedFile};
use crate::scanner::FileInfo;

thread_local! {
    static LOCAL_PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

/// A pool of tree-sitter parsers for parallel file parsing.
pub struct ParserPool {
    registry: Arc<GrammarRegistry>,
    thread_pool: rayon::ThreadPool,
    /// Approximate byte budget per batch chunk (~20 MB).
    pub chunk_budget: usize,
}

impl ParserPool {
    /// Create a new parser pool with the given number of threads.
    pub fn new(num_threads: usize) -> anyhow::Result<Self> {
        let thread_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .build()?;
        Ok(Self {
            registry: Arc::new(GrammarRegistry::new()),
            thread_pool,
            chunk_budget: 20 * 1024 * 1024,
        })
    }

    /// Parse a batch of files in parallel.
    pub fn parse_batch(&self, files: &[FileInfo]) -> Vec<ParsedFile> {
        self.thread_pool.install(|| {
            files
                .par_chunks(self.chunk_size(files))
                .flat_map(|chunk| {
                    chunk
                        .iter()
                        .filter_map(|file| self.parse_file(file))
                        .collect::<Vec<_>>()
                })
                .collect()
        })
    }

    /// Parse a single file.
    pub fn parse_file(&self, file: &FileInfo) -> Option<ParsedFile> {
        let language = file.language?;
        let grammar = self.registry.get(language)?;

        let source = std::fs::read(&file.path).ok()?;

        let source_to_parse = source.clone();

        let tree = LOCAL_PARSER.with(|p| {
            let mut parser = p.borrow_mut();
            parser.set_language(&grammar).ok()?;
            parser.parse(&source_to_parse, None)
        })?;

        Some(ParsedFile {
            file_path: file.path.clone(),
            language,
            tree,
            source: source_to_parse,
        })
    }

    /// Pre-load grammars for the given languages.
    pub fn preload_grammars(&self, langs: &[Language]) {
        self.registry.preload(langs);
    }

    /// Compute chunk size based on byte budget.
    fn chunk_size(&self, files: &[FileInfo]) -> usize {
        if files.is_empty() {
            return 1;
        }
        let total_size: u64 = files.iter().map(|f| f.size).sum();
        let num_chunks = (total_size as usize / self.chunk_budget).max(1);
        (files.len() / num_chunks).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_file(path: &str, content: &str, lang: Language) -> (FileInfo, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join(path);
        std::fs::write(&file_path, content).unwrap();
        let info = FileInfo {
            path: file_path.clone(),
            relative_path: PathBuf::from(path),
            size: content.len() as u64,
            language: Some(lang),
        };
        (info, tmp)
    }

    #[test]
    fn parse_rust_file() {
        let pool = ParserPool::new(2).unwrap();
        let (file, _tmp) = make_file(
            "main.rs",
            "fn main() { println!(\"hello\"); }",
            Language::Rust,
        );
        let parsed = pool.parse_file(&file).unwrap();
        assert_eq!(parsed.language, Language::Rust);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn parse_typescript_file() {
        let pool = ParserPool::new(2).unwrap();
        let (file, _tmp) = make_file(
            "app.ts",
            "function greet(name: string): string { return `Hello ${name}`; }",
            Language::TypeScript,
        );
        let parsed = pool.parse_file(&file).unwrap();
        assert_eq!(parsed.language, Language::TypeScript);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn parse_python_file() {
        let pool = ParserPool::new(2).unwrap();
        let (file, _tmp) = make_file(
            "main.py",
            "def main():\n    print('hello')\n",
            Language::Python,
        );
        let parsed = pool.parse_file(&file).unwrap();
        assert_eq!(parsed.language, Language::Python);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn parse_batch_parallel() {
        let pool = ParserPool::new(4).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut files = Vec::new();

        for i in 0..8 {
            let path = tmp.path().join(format!("file{}.rs", i));
            let content = format!("fn func{}() {{}}\n", i);
            std::fs::write(&path, &content).unwrap();
            files.push(FileInfo {
                path,
                relative_path: PathBuf::from(format!("file{}.rs", i)),
                size: content.len() as u64,
                language: Some(Language::Rust),
            });
        }

        let parsed = pool.parse_batch(&files);
        assert_eq!(parsed.len(), 8);
    }
}

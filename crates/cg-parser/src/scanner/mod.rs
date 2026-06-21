//! File system scanner based on the `ignore` crate (ripgrep core).
//!
//! Supports `.gitignore`, hard-coded blacklist, file size limits,
//! parallel directory traversal, and language inference.

use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use cg_common::Language;

/// Default maximum file size: 512 KB.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 512 * 1024;

// ============================================================================
// Blacklists (mirroring GitNexus defaults)
// ============================================================================

/// Directories to always skip.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "dist",
    "build",
    "target",
    "__pycache__",
    ".vscode",
    "vendor",
    "coverage",
    "tmp",
    "logs",
    ".idea",
    ".history",
    ".husky",
    ".github",
    ".claude",
    ".cursor",
    ".sisyphus",
];

/// File extensions to always skip.
const IGNORED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "zip", "tar", "gz", "bz2", "xz", "7z",
    "rar", "exe", "dll", "so", "dylib", "wasm", "node", "jar", "pdf", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "mp3", "mp4", "avi", "mov", "wmv", "flv", "webm", "woff", "woff2", "ttf", "otf",
    "eot", "db", "sqlite", "sqlite3", "lock", "min.js", "min.css", "map",
];

/// Exact file names to always skip.
const IGNORED_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "Cargo.lock",
    "pnpm-lock.yaml",
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    "LICENSE",
    "CHANGELOG.md",
    "Thumbs.db",
    ".DS_Store",
    ".git-blame-ignore-revs",
    ".gitattributes",
    ".prettierrc",
    ".eslintignore",
    ".dockerignore",
];

/// Check whether a path should be ignored by hard-coded rules.
pub fn should_ignore_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");

    // Check directory components
    for part in normalized.split('/') {
        if IGNORED_DIRS.iter().any(|&d| d.eq_ignore_ascii_case(part)) {
            return true;
        }
    }

    // Check exact file name
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if IGNORED_FILES.iter().any(|&f| f.eq_ignore_ascii_case(name)) {
            return true;
        }
        // Generated file patterns
        if name.contains(".bundle.")
            || name.contains(".chunk.")
            || name.contains(".generated.")
            || name.ends_with(".d.ts")
        {
            return true;
        }
    }

    // Check extension (support compound extensions like .min.js)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        for &ext in IGNORED_EXTENSIONS {
            if name.ends_with(&format!(".{}", ext)) || name.eq_ignore_ascii_case(ext) {
                return true;
            }
        }
    }

    false
}

// ============================================================================
// FileInfo
// ============================================================================

/// Information about a discovered source file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Path relative to the scan root.
    pub relative_path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Detected language (if any).
    pub language: Option<Language>,
}

impl FileInfo {
    /// Create a new FileInfo by resolving the path relative to the scan root.
    pub fn from_path(path: &Path, root: &Path) -> anyhow::Result<Self> {
        let size = std::fs::metadata(path)?.len();
        let relative_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        let ext = path.extension().and_then(|e| e.to_str());
        let language = ext.and_then(Language::from_extension);
        Ok(Self {
            path: path.to_path_buf(),
            relative_path,
            size,
            language,
        })
    }
}

// ============================================================================
// ScanOptions
// ============================================================================

/// Configuration for the directory scanner.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Maximum file size in bytes (files larger than this are skipped).
    pub max_file_size: u64,
    /// Whether to respect `.gitignore` files.
    pub respect_gitignore: bool,
    /// Whether to include hidden files/directories.
    pub include_hidden: bool,
    /// Additional custom ignore patterns (gitignore syntax).
    pub custom_ignore: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            respect_gitignore: true,
            include_hidden: false,
            custom_ignore: Vec::new(),
        }
    }
}

// ============================================================================
// ScanResult
// ============================================================================

/// Result of a directory scan.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Discovered files.
    pub files: Vec<FileInfo>,
    /// Set of all relative paths (files only).
    pub all_paths: HashSet<PathBuf>,
    /// Number of files skipped due to size.
    pub skipped_large: usize,
    /// Number of files skipped due to ignore rules.
    pub skipped_ignored: usize,
    /// Total files seen (before filtering).
    pub total_seen: usize,
}

// ============================================================================
// Scanner
// ============================================================================

/// Scan a directory respecting `.gitignore` and blacklist rules.
pub fn scan_directory(root: &Path, options: &ScanOptions) -> anyhow::Result<ScanResult> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(!options.include_hidden)
        .git_ignore(options.respect_gitignore)
        .git_global(options.respect_gitignore)
        .git_exclude(options.respect_gitignore)
        .require_git(false);

    // Add custom ignore patterns
    if !options.custom_ignore.is_empty() {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        for pattern in &options.custom_ignore {
            overrides.add(&format!("!{}", pattern))?;
        }
        builder.overrides(overrides.build()?);
    }

    let walker = builder.build();

    let mut files = Vec::new();
    let mut skipped_large = 0usize;
    let mut skipped_ignored = 0usize;
    let mut total_seen = 0usize;

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        total_seen += 1;
        let path = entry.path();

        // Hard-coded blacklist (check relative path so /tmp doesn't match "tmp")
        let rel_path = path.strip_prefix(root).unwrap_or(path);
        if should_ignore_path(rel_path) {
            skipped_ignored += 1;
            continue;
        }

        // Size limit
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        if size > options.max_file_size {
            skipped_large += 1;
            continue;
        }

        let file_info = FileInfo::from_path(path, root)?;
        files.push(file_info);
    }

    let all_paths: HashSet<PathBuf> = files.iter().map(|f| f.relative_path.clone()).collect();

    Ok(ScanResult {
        files,
        all_paths,
        skipped_large,
        skipped_ignored,
        total_seen,
    })
}

/// Parallel variant: scan multiple roots in parallel.
pub fn scan_directories_parallel(
    roots: &[&Path],
    options: &ScanOptions,
) -> anyhow::Result<Vec<ScanResult>> {
    roots
        .par_iter()
        .map(|&root| scan_directory(root, options))
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_this_project() {
        let options = ScanOptions::default();
        let result = scan_directory(Path::new("."), &options).unwrap();
        assert!(!result.files.is_empty());
        assert!(
            result
                .all_paths
                .iter()
                .any(|p| p.extension().map(|e| e == "rs").unwrap_or(false))
        );
    }

    #[test]
    fn ignores_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        let node_modules = root.join("node_modules/foo");
        std::fs::create_dir_all(&node_modules).unwrap();
        std::fs::write(node_modules.join("index.js"), "").unwrap();

        let options = ScanOptions::default();
        let result = scan_directory(root, &options).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("main.rs"));
        assert!(result.skipped_ignored > 0);
    }

    #[test]
    fn skips_large_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let opts = ScanOptions {
            max_file_size: 10, // 10 bytes
            ..ScanOptions::default()
        };

        std::fs::write(root.join("small.rs"), "a").unwrap();
        std::fs::write(root.join("large.rs"), "x".repeat(100)).unwrap();

        let result = scan_directory(root, &opts).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("small.rs"));
        assert_eq!(result.skipped_large, 1);
    }

    #[test]
    fn ignores_lock_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("Cargo.lock"), "[[package]]").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();

        let options = ScanOptions::default();
        let result = scan_directory(root, &options).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("main.rs"));
    }

    #[test]
    fn language_inference() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("main.rs"), "").unwrap();
        std::fs::write(root.join("app.ts"), "").unwrap();
        std::fs::write(root.join("lib.py"), "").unwrap();
        std::fs::write(root.join("README"), "").unwrap(); // no extension

        let options = ScanOptions::default();
        let result = scan_directory(root, &options).unwrap();
        assert_eq!(result.files.len(), 4);

        let langs: Vec<_> = result.files.iter().filter_map(|f| f.language).collect();
        assert!(langs.contains(&Language::Rust));
        assert!(langs.contains(&Language::TypeScript));
        assert!(langs.contains(&Language::Python));
    }

    #[test]
    fn scan_respects_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(root.join("main.rs"), "").unwrap();
        std::fs::write(root.join(".gitignore"), "secret.rs\n").unwrap();
        std::fs::write(root.join("secret.rs"), "").unwrap();

        let options = ScanOptions::default();
        let result = scan_directory(root, &options).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].relative_path, PathBuf::from("main.rs"));
    }

    #[test]
    fn parallel_scan_multiple_roots() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();

        std::fs::write(tmp1.path().join("a.rs"), "").unwrap();
        std::fs::write(tmp2.path().join("b.rs"), "").unwrap();

        let options = ScanOptions::default();
        let roots: Vec<&Path> = vec![tmp1.path(), tmp2.path()];
        let results = scan_directories_parallel(&roots, &options).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].files.len(), 1);
        assert_eq!(results[1].files.len(), 1);
    }
}

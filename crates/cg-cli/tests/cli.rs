//! End-to-end tests for the `codegraph` CLI binary.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn codegraph() -> Command {
    Command::cargo_bin("codegraph").unwrap()
}

fn temp_repo_with_file(file_name: &str, content: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(file_name), content).unwrap();
    tmp
}

#[test]
fn help_shows_subcommands() {
    codegraph()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("analyze"))
        .stdout(predicate::str::contains("query"))
        .stdout(predicate::str::contains("status"));
}

#[test]
fn analyze_creates_index_metadata() {
    let tmp = temp_repo_with_file(
        "main.rs",
        r#"
fn main() {
    println!("hello");
}

fn greet(name: &str) {
    println!("hello {}", name);
}
"#,
    );

    let registry = TempDir::new().unwrap();
    codegraph()
        .arg("analyze")
        .arg(tmp.path())
        .env("CODEGRAPH_REGISTRY", registry.path().join("registry.json"))
        .assert()
        .success();

    let meta = tmp.path().join(".codegraph/meta.json");
    assert!(meta.exists(), "meta.json should be created after analyze");

    let content = fs::read_to_string(&meta).unwrap();
    let value: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(value["stats"]["nodes"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn status_reports_indexed_repo() {
    let tmp = temp_repo_with_file("main.rs", "fn main() {}");
    let registry = TempDir::new().unwrap();

    codegraph()
        .arg("analyze")
        .arg(tmp.path())
        .env("CODEGRAPH_REGISTRY", registry.path().join("registry.json"))
        .assert()
        .success();

    codegraph()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Nodes:"));
}

#[test]
fn query_finds_symbol() {
    let tmp = temp_repo_with_file(
        "lib.rs",
        r#"
pub fn compute() -> i32 {
    42
}
"#,
    );
    let registry = TempDir::new().unwrap();

    codegraph()
        .arg("analyze")
        .arg(tmp.path())
        .env("CODEGRAPH_REGISTRY", registry.path().join("registry.json"))
        .assert()
        .success();

    codegraph()
        .arg("query")
        .arg("compute")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("compute"));
}

#[test]
fn clean_force_removes_index() {
    let tmp = temp_repo_with_file("main.rs", "fn main() {}");
    let registry = TempDir::new().unwrap();

    codegraph()
        .arg("analyze")
        .arg(tmp.path())
        .env("CODEGRAPH_REGISTRY", registry.path().join("registry.json"))
        .assert()
        .success();

    assert!(tmp.path().join(".codegraph").exists());

    codegraph()
        .arg("clean")
        .arg("--force")
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(!tmp.path().join(".codegraph").exists());
}

#[test]
fn clean_all_force_clears_registry() {
    let tmp = temp_repo_with_file("main.rs", "fn main() {}");
    let registry = TempDir::new().unwrap();
    let registry_path: PathBuf = registry.path().join("registry.json");

    codegraph()
        .arg("analyze")
        .arg(tmp.path())
        .env("CODEGRAPH_REGISTRY", &registry_path)
        .assert()
        .success();

    assert!(registry_path.exists());

    codegraph()
        .arg("clean")
        .arg("--all")
        .arg("--force")
        .env("CODEGRAPH_REGISTRY", &registry_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleaned 1 repositories"));

    assert!(!tmp.path().join(".codegraph").exists());
    let content = fs::read_to_string(&registry_path).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&content).unwrap();
    assert!(entries.is_empty());
}

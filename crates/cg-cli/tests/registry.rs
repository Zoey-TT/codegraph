//! Integration tests for the global repository registry.

use cg_cli::registry::{load_registry, register_repo, unregister_repo};
use std::sync::Mutex;
use tempfile::TempDir;

// Tests mutate the process-wide CODEGRAPH_REGISTRY environment variable, so
// they must run sequentially.
static REGISTRY_SERIAL: Mutex<()> = Mutex::new(());

fn with_registry<T>(test: T)
where
    T: FnOnce(),
{
    let _guard = REGISTRY_SERIAL.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("registry.json");
    unsafe {
        std::env::set_var("CODEGRAPH_REGISTRY", &path);
    }
    test();
    unsafe {
        std::env::remove_var("CODEGRAPH_REGISTRY");
    }
}

#[test]
fn register_and_load_roundtrip() {
    with_registry(|| {
        let repo = TempDir::new().unwrap();
        register_repo(repo.path(), Some(10), Some(20)).unwrap();

        let entries = load_registry().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].name,
            repo.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(entries[0].node_count, Some(10));
        assert_eq!(entries[0].edge_count, Some(20));
    });
}

#[test]
fn unregister_removes_entry() {
    with_registry(|| {
        let repo = TempDir::new().unwrap();
        register_repo(repo.path(), Some(1), Some(2)).unwrap();
        assert_eq!(load_registry().unwrap().len(), 1);

        unregister_repo(repo.path()).unwrap();
        assert!(load_registry().unwrap().is_empty());
    });
}

#[test]
fn register_updates_existing_entry() {
    with_registry(|| {
        let repo = TempDir::new().unwrap();
        register_repo(repo.path(), Some(1), Some(2)).unwrap();
        register_repo(repo.path(), Some(3), Some(4)).unwrap();

        let entries = load_registry().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].node_count, Some(3));
        assert_eq!(entries[0].edge_count, Some(4));
    });
}

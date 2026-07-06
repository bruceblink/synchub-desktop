use synchub_desktop::client::normalize_base_url;
use synchub_desktop::models::{
    Manifest, ManifestEntry, WorkspaceRegistryEntry, WorkspaceSnapshot, format_bytes,
    is_success_code, workspace_metrics,
};

#[test]
fn base_url_is_normalized() {
    assert_eq!(normalize_base_url("localhost:8765/"), "http://localhost:8765");
    assert_eq!(normalize_base_url("https://sync.likanug.app/"), "https://sync.likanug.app");
}

#[test]
fn workspace_metrics_count_manifest_versions() {
    let snapshot = WorkspaceSnapshot {
        entry: WorkspaceRegistryEntry {
            root: "F:/work/notes".to_string(),
            remote_path: "/notes".to_string(),
            ..WorkspaceRegistryEntry::default()
        },
        manifest: Some(Manifest {
            items: vec![
                ManifestEntry { remote_version: Some(1), ..ManifestEntry::default() },
                ManifestEntry { remote_version: None, ..ManifestEntry::default() },
            ],
            ..Manifest::default()
        }),
        trash_entries: 3,
        ..WorkspaceSnapshot::default()
    };

    let metrics = workspace_metrics(&snapshot);
    assert_eq!(metrics.manifest_files, 2);
    assert_eq!(metrics.remote_tracked, 1);
    assert_eq!(metrics.local_only, 1);
    assert_eq!(metrics.trash_entries, 3);
}

#[test]
fn api_success_codes_match_synchub_envelope() {
    assert!(is_success_code(&serde_json::json!(0)));
    assert!(is_success_code(&serde_json::json!("0")));
    assert!(is_success_code(&serde_json::Value::Null));
    assert!(!is_success_code(&serde_json::json!("FILE_CONFLICT")));
}

#[test]
fn bytes_are_human_readable() {
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(2048), "2.0 KB");
}

use synchub_desktop::client::normalize_base_url;
use synchub_desktop::models::{
    Manifest, ManifestEntry, WorkspaceRegistryEntry, WorkspaceSnapshot, conflict_resolution_label,
    format_bytes, is_success_code, workspace_metrics,
};
use synchub_desktop::sync_commands::{
    parse_workspace_paths, sync_command_args, workspace_init_command_args,
};

#[test]
fn base_url_is_normalized() {
    assert_eq!(
        normalize_base_url("localhost:8765/"),
        "http://localhost:8765"
    );
    assert_eq!(
        normalize_base_url("https://sync.likanug.app/"),
        "https://sync.likanug.app"
    );
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
                ManifestEntry {
                    remote_version: Some(1),
                    ..ManifestEntry::default()
                },
                ManifestEntry {
                    remote_version: None,
                    ..ManifestEntry::default()
                },
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

#[test]
fn conflict_resolution_labels_are_human_readable() {
    assert_eq!(conflict_resolution_label("pending"), "pending");
    assert_eq!(conflict_resolution_label("keep_local"), "keep local");
    assert_eq!(conflict_resolution_label("keep_remote"), "keep remote");
    assert_eq!(conflict_resolution_label("keep_both"), "keep both");
    assert_eq!(conflict_resolution_label("other"), "unknown");
}

#[test]
fn sync_command_args_include_workspace_and_config() {
    assert_eq!(
        sync_command_args("dry-run", "C:/work", "C:/cfg/config.json").expect("dry-run args"),
        vec![
            "sync",
            "once",
            "--dry-run",
            "--path",
            "C:/work",
            "--config",
            "C:/cfg/config.json",
        ]
    );
}

#[test]
fn status_sync_command_includes_remote_context() {
    assert_eq!(
        sync_command_args("status", "C:/work", "C:/cfg/config.json").expect("status args"),
        vec![
            "sync",
            "status",
            "--show-remote",
            "--show-conflicts",
            "--path",
            "C:/work",
            "--config",
            "C:/cfg/config.json",
        ]
    );
}

#[test]
fn unknown_sync_command_is_rejected() {
    assert!(sync_command_args("bogus", "C:/work", "C:/cfg/config.json").is_none());
}

#[test]
fn workspace_paths_are_split_for_batch_init() {
    assert_eq!(
        parse_workspace_paths("C:/work/notes\n\"D:/work/code\"; E:/work/docs "),
        vec!["C:/work/notes", "D:/work/code", "E:/work/docs"]
    );
}

#[test]
fn workspace_init_args_support_multiple_paths() {
    let roots = vec!["C:/work/notes".to_string(), "D:/work/code".to_string()];

    assert_eq!(
        workspace_init_command_args(&roots, "/workspace", "C:/cfg/config.json")
            .expect("workspace init args"),
        vec![
            "workspace",
            "init",
            "--path",
            "C:/work/notes",
            "--path",
            "D:/work/code",
            "--remote-root",
            "/workspace",
            "--config",
            "C:/cfg/config.json",
        ]
    );
}

#[test]
fn workspace_init_args_reject_empty_paths() {
    assert!(workspace_init_command_args(&[], "", "C:/cfg/config.json").is_none());
}

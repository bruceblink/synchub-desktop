use synchub_desktop::client::normalize_base_url;
use synchub_desktop::config::{
    DesktopSettings, load_cli_config, load_settings_from_paths, load_workspace_registry,
    update_cli_server_url, update_workspace_server_urls, write_json,
};
use synchub_desktop::models::{
    ApiEnvelope, ApiStatus, CliConfig, Device, FileVersion, Manifest, ManifestEntry,
    WorkspaceConfig, WorkspaceRegistry, WorkspaceRegistryEntry, WorkspaceSnapshot,
    compose_remote_directory_path, conflict_resolution_label, file_belongs_to_remote_root,
    file_version_label, format_bytes, is_current_device, is_file_version_pinned, is_success_code,
    pending_manifest_changes, workspace_metrics,
};
use synchub_desktop::sync_commands::{
    daemon_command_args, file_download_command_args, manifest_scan_command_args,
    parse_workspace_paths, sync_command_args, trash_list_command_args, trash_restore_command_args,
    workspace_init_command_args, workspace_prune_command_args, workspace_remove_command_args,
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
fn logged_in_server_updates_registered_workspace_configs() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-server-update-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_root = root.join("workspace");
    let workspace_config_path = workspace_root.join(".synchub").join("workspace.json");
    let registry_path = root.join("workspaces.json");

    write_json(
        &workspace_config_path,
        &WorkspaceConfig {
            server_url: "https://old.example".to_string(),
            ..WorkspaceConfig::default()
        },
    )
    .expect("write workspace config");
    write_json(
        &registry_path,
        &WorkspaceRegistry {
            version: 1,
            workspaces: vec![WorkspaceRegistryEntry {
                root: workspace_root.display().to_string(),
                workspace_config_path: workspace_config_path.display().to_string(),
                server_url: "https://old.example".to_string(),
                ..WorkspaceRegistryEntry::default()
            }],
            ..WorkspaceRegistry::default()
        },
    )
    .expect("write workspace registry");

    assert_eq!(
        update_workspace_server_urls(&registry_path, "https://sync.likanug.app")
            .expect("update workspace servers"),
        1
    );
    let config: WorkspaceConfig = serde_json::from_str(
        &std::fs::read_to_string(&workspace_config_path).expect("read workspace config"),
    )
    .expect("decode workspace config");
    let registry = load_workspace_registry(&registry_path).expect("load workspace registry");
    assert_eq!(config.server_url, "https://sync.likanug.app");
    assert_eq!(
        registry.workspaces[0].server_url,
        "https://sync.likanug.app"
    );

    std::fs::remove_dir_all(root).expect("remove temp files");
}

#[test]
fn desktop_server_updates_cli_config_without_losing_login() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-cli-server-update-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let config_path = root.join("config.json");
    let original = CliConfig {
        server_url: "https://old.example".to_string(),
        user: synchub_desktop::models::User {
            id: "user-1".to_string(),
            email: "user@example.com".to_string(),
            status: "active".to_string(),
        },
        tokens: synchub_desktop::models::TokenPair {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_in: 900,
        },
        ..CliConfig::default()
    };
    write_json(&config_path, &original).expect("write CLI config");

    let updated = update_cli_server_url(&config_path, "https://sync.likanug.app")
        .expect("update CLI server")
        .expect("existing CLI config");
    let saved = load_cli_config(&config_path)
        .expect("load CLI config")
        .expect("saved CLI config");
    assert_eq!(updated.server_url, "https://sync.likanug.app");
    assert_eq!(saved.server_url, "https://sync.likanug.app");
    assert_eq!(saved.user, original.user);
    assert_eq!(saved.tokens, original.tokens);

    std::fs::remove_dir_all(root).expect("remove temp files");
}

#[test]
fn desktop_settings_are_independent_from_legacy_cli_config() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-settings-priority-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let settings_path = root.join("settings.json");
    let config_path = root.join("config.json");
    write_json(
        &config_path,
        &CliConfig {
            server_url: "https://legacy-cli.example".to_string(),
            ..CliConfig::default()
        },
    )
    .expect("write legacy CLI config");

    assert_eq!(
        load_settings_from_paths(&settings_path, &config_path).server_url,
        "https://legacy-cli.example"
    );
    write_json(
        &settings_path,
        &DesktopSettings {
            server_url: "https://desktop.example".to_string(),
        },
    )
    .expect("write desktop settings");
    assert_eq!(
        load_settings_from_paths(&settings_path, &config_path).server_url,
        "https://desktop.example"
    );

    std::fs::remove_dir_all(root).expect("remove temp files");
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
fn pending_manifest_changes_count_created_updated_and_deleted_files() {
    let root = std::env::temp_dir().join(format!("synchub-desktop-pending-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".synchub")).expect("create metadata dir");
    std::fs::create_dir_all(root.join("docs")).expect("create docs dir");
    std::fs::write(root.join("updated.txt"), "new").expect("write updated");
    std::fs::write(root.join("docs").join("created.txt"), "created").expect("write created");
    std::fs::write(root.join(".synchub").join("ignored.txt"), "ignored")
        .expect("write metadata file");

    let snapshot = WorkspaceSnapshot {
        entry: WorkspaceRegistryEntry {
            root: root.display().to_string(),
            remote_path: "/notes".to_string(),
            ..WorkspaceRegistryEntry::default()
        },
        manifest: Some(Manifest {
            items: vec![
                ManifestEntry {
                    relative_path: "updated.txt".to_string(),
                    size: 3,
                    sha256: "old-hash".to_string(),
                    ..ManifestEntry::default()
                },
                ManifestEntry {
                    relative_path: "deleted.txt".to_string(),
                    size: 7,
                    sha256: "deleted-hash".to_string(),
                    ..ManifestEntry::default()
                },
            ],
            ..Manifest::default()
        }),
        ..WorkspaceSnapshot::default()
    };

    let changes = pending_manifest_changes(&snapshot);
    assert_eq!(changes.created, 1);
    assert_eq!(changes.updated, 1);
    assert_eq!(changes.deleted, 1);
    assert_eq!(changes.total(), 3);

    std::fs::remove_dir_all(root).expect("remove temp workspace");
}

#[test]
fn api_success_codes_match_synchub_envelope() {
    assert!(is_success_code(&serde_json::json!(0)));
    assert!(is_success_code(&serde_json::json!("0")));
    assert!(is_success_code(&serde_json::Value::Null));
    assert!(!is_success_code(&serde_json::json!("FILE_CONFLICT")));
}

#[test]
fn readiness_status_keeps_component_checks() {
    let envelope: ApiEnvelope<ApiStatus> = serde_json::from_str(
        r#"{
            "code": 0,
            "message": "ok",
            "data": {
                "status": "ready",
                "checks": {
                    "database": { "status": "ready" },
                    "storage": { "status": "ready" }
                }
            }
        }"#,
    )
    .expect("readiness envelope");
    let status = envelope.data.expect("readiness data");

    assert_eq!(status.status, "ready");
    assert_eq!(status.checks["database"].status, "ready");
    assert_eq!(status.checks["storage"].status, "ready");
}

#[test]
fn bytes_are_human_readable() {
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(2048), "2.0 KB");
}

#[test]
fn cloud_trash_is_scoped_to_the_workspace_remote_root() {
    assert!(file_belongs_to_remote_root("/notes/deleted.txt", "/notes"));
    assert!(file_belongs_to_remote_root("/notes", "/notes"));
    assert!(!file_belongs_to_remote_root(
        "/notebook/deleted.txt",
        "/notes"
    ));
    assert!(file_belongs_to_remote_root("/anything", "/"));
}

#[test]
fn file_version_helpers_are_human_readable() {
    let pinned = FileVersion {
        version: 7,
        pinned_at: Some("2026-07-07T00:00:00Z".to_string()),
        ..FileVersion::default()
    };
    let unpinned = FileVersion {
        version: 8,
        pinned_at: None,
        ..FileVersion::default()
    };

    assert_eq!(file_version_label(&pinned), "v7");
    assert!(is_file_version_pinned(&pinned));
    assert!(!is_file_version_pinned(&unpinned));
}

#[test]
fn remote_directory_path_is_composed_from_workspace_base() {
    assert_eq!(
        compose_remote_directory_path("docs/notes", "/workspace").as_deref(),
        Some("/workspace/docs/notes")
    );
    assert_eq!(
        compose_remote_directory_path("/archive/docs", "/workspace").as_deref(),
        Some("/archive/docs")
    );
    assert_eq!(
        compose_remote_directory_path("../secret", "/workspace"),
        None
    );
    assert_eq!(compose_remote_directory_path("/", "/workspace"), None);
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

#[test]
fn workspace_remove_args_unregister_selected_path() {
    assert_eq!(
        workspace_remove_command_args("C:/work/notes", "C:/cfg/config.json")
            .expect("workspace remove args"),
        vec![
            "workspace",
            "remove",
            "--path",
            "C:/work/notes",
            "--config",
            "C:/cfg/config.json",
            "--json",
        ]
    );
}

#[test]
fn workspace_remove_args_reject_empty_path() {
    assert!(workspace_remove_command_args("", "C:/cfg/config.json").is_none());
}

#[test]
fn workspace_prune_args_use_json_output() {
    assert_eq!(
        workspace_prune_command_args("C:/cfg/config.json"),
        vec![
            "workspace",
            "prune",
            "--config",
            "C:/cfg/config.json",
            "--json",
        ]
    );
}

#[test]
fn manifest_scan_args_write_json_result() {
    assert_eq!(
        manifest_scan_command_args("C:/work", "C:/work/.synchub/workspace.json")
            .expect("manifest scan args"),
        vec![
            "manifest",
            "scan",
            "--path",
            "C:/work",
            "--workspace-config",
            "C:/work/.synchub/workspace.json",
            "--json",
        ]
    );
}

#[test]
fn manifest_scan_args_reject_empty_path() {
    assert!(manifest_scan_command_args("", "C:/work/.synchub/workspace.json").is_none());
}

#[test]
fn file_download_args_use_file_id_and_json_output() {
    assert_eq!(
        file_download_command_args(
            "C:/work",
            "C:/work/.synchub/workspace.json",
            "C:/cfg/config.json",
            "file_1",
        )
        .expect("file download args"),
        vec![
            "file",
            "download",
            "--path",
            "C:/work",
            "--workspace-config",
            "C:/work/.synchub/workspace.json",
            "--config",
            "C:/cfg/config.json",
            "--file-id",
            "file_1",
            "--json",
        ]
    );
}

#[test]
fn file_download_args_reject_empty_file_id() {
    assert!(
        file_download_command_args("C:/work", "C:/work/.synchub/workspace.json", "", "").is_none()
    );
}

#[test]
fn trash_list_args_use_json_output() {
    assert_eq!(
        trash_list_command_args("C:/work", "C:/work/.synchub/workspace.json", 25)
            .expect("trash list args"),
        vec![
            "sync",
            "trash",
            "--path",
            "C:/work",
            "--workspace-config",
            "C:/work/.synchub/workspace.json",
            "--limit",
            "25",
            "--json",
        ]
    );
}

#[test]
fn trash_restore_args_include_batch_and_entry() {
    assert_eq!(
        trash_restore_command_args(
            "C:/work",
            "C:/work/.synchub/workspace.json",
            "20260702T010000.000000000Z",
            "/docs/readme.md/",
        )
        .expect("trash restore args"),
        vec![
            "sync",
            "trash",
            "restore",
            "--path",
            "C:/work",
            "--workspace-config",
            "C:/work/.synchub/workspace.json",
            "--batch",
            "20260702T010000.000000000Z",
            "--entry",
            "docs/readme.md",
        ]
    );
}

#[test]
fn daemon_reset_state_args_target_selected_workspace() {
    assert_eq!(
        daemon_command_args("reset-state", "C:/work", "C:/cfg/config.json")
            .expect("daemon reset args"),
        vec![
            "sync",
            "daemon",
            "--reset-state",
            "--path",
            "C:/work",
            "--config",
            "C:/cfg/config.json",
        ]
    );
}

#[test]
fn daemon_args_reject_unknown_action() {
    assert!(daemon_command_args("restart", "C:/work", "C:/cfg/config.json").is_none());
}

#[test]
fn current_device_matches_workspace_device_id() {
    let device = Device {
        id: "dev_1".to_string(),
        ..Device::default()
    };
    let snapshot = WorkspaceSnapshot {
        config: Some(WorkspaceConfig {
            device_id: Some("dev_1".to_string()),
            ..WorkspaceConfig::default()
        }),
        ..WorkspaceSnapshot::default()
    };

    assert!(is_current_device(&device, &snapshot));
}

#[test]
fn blank_workspace_device_id_does_not_match_device() {
    let device = Device {
        id: "dev_1".to_string(),
        ..Device::default()
    };

    assert!(!is_current_device(&device, &WorkspaceSnapshot::default()));
}

use sha2::Digest;
use synchub_desktop::client::normalize_base_url;
use synchub_desktop::config::{
    DesktopSettings, initialize_workspaces, load_cli_config, load_settings_from_paths,
    load_workspace_registry, prune_workspace_registrations, remove_workspace_registration,
    update_cli_server_url, update_workspace_server_urls, write_json,
};
use synchub_desktop::models::{
    ApiEnvelope, ApiStatus, CliConfig, Device, FileVersion, Manifest, ManifestEntry,
    WorkspaceConfig, WorkspaceRegistry, WorkspaceRegistryEntry, WorkspaceSnapshot,
    compose_remote_directory_path, conflict_resolution_label, file_belongs_to_remote_root,
    file_version_label, format_bytes, is_current_device, is_file_version_pinned, is_success_code,
    pending_manifest_changes, workspace_metrics,
};
use synchub_desktop::native_manifest::scan_and_save_manifest;
use synchub_desktop::sync_commands::{
    daemon_command_args, parse_workspace_paths, sync_command_args,
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
fn native_manifest_scan_preserves_versions_and_applies_ignores() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-native-manifest-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".synchub")).expect("create metadata directory");
    std::fs::create_dir_all(root.join("docs")).expect("create docs directory");
    std::fs::create_dir_all(root.join("build")).expect("create ignored directory");
    std::fs::write(root.join("docs").join("same.txt"), "same").expect("write same file");
    std::fs::write(root.join("changed.txt"), "new").expect("write changed file");
    std::fs::write(root.join("draft.tmp"), "ignored").expect("write ignored file");
    std::fs::write(root.join("build").join("output.bin"), "ignored")
        .expect("write ignored directory file");
    std::fs::write(root.join(".synchub").join("secret"), "metadata").expect("write metadata file");
    std::fs::write(root.join(".synchubignore"), "*.tmp\nbuild/\n").expect("write ignore file");

    let snapshot = WorkspaceSnapshot {
        entry: WorkspaceRegistryEntry {
            root: root.display().to_string(),
            remote_path: "/workspace".to_string(),
            ..WorkspaceRegistryEntry::default()
        },
        manifest: Some(Manifest {
            items: vec![
                ManifestEntry {
                    relative_path: "docs/same.txt".to_string(),
                    size: 4,
                    sha256: format!("{:x}", sha2::Sha256::digest(b"same")),
                    remote_version: Some(7),
                    ..ManifestEntry::default()
                },
                ManifestEntry {
                    relative_path: "changed.txt".to_string(),
                    size: 3,
                    sha256: format!("{:x}", sha2::Sha256::digest(b"old")),
                    remote_version: Some(4),
                    ..ManifestEntry::default()
                },
            ],
            ..Manifest::default()
        }),
        ..WorkspaceSnapshot::default()
    };

    let manifest = scan_and_save_manifest(&snapshot).expect("scan native manifest");
    let paths = manifest
        .items
        .iter()
        .map(|item| item.relative_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![".synchubignore", "changed.txt", "docs/same.txt"]
    );
    assert_eq!(manifest.items[1].remote_version, None);
    assert_eq!(manifest.items[2].remote_version, Some(7));
    assert_eq!(manifest.items[2].path, "/workspace/docs/same.txt");
    let saved: Manifest = serde_json::from_str(
        &std::fs::read_to_string(root.join(".synchub").join("manifest.json"))
            .expect("read saved manifest"),
    )
    .expect("decode saved manifest");
    assert_eq!(saved, manifest);

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
fn desktop_manages_workspace_registry_without_cli() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-workspace-management-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_root = root.join("notes");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let registry_path = root.join("workspaces.json");
    let legacy_config_path = root.join("config.json");
    let login = CliConfig {
        server_url: "https://sync.likanug.app".to_string(),
        user: synchub_desktop::models::User {
            id: "user-1".to_string(),
            email: "user@example.com".to_string(),
            ..Default::default()
        },
        ..CliConfig::default()
    };

    let initialized = initialize_workspaces(
        &[workspace_root.display().to_string()],
        "/documents",
        &login,
        &registry_path,
        &legacy_config_path,
    )
    .expect("initialize workspace");
    assert_eq!(initialized.len(), 1);
    assert_eq!(initialized[0].remote_path, "/documents/notes");

    let workspace_config: WorkspaceConfig = serde_json::from_str(
        &std::fs::read_to_string(workspace_root.join(".synchub/workspace.json"))
            .expect("read workspace config"),
    )
    .expect("decode workspace config");
    assert_eq!(workspace_config.server_url, login.server_url);
    assert_eq!(workspace_config.user_id, login.user.id);

    assert_eq!(
        prune_workspace_registrations(&registry_path).expect("prune registry"),
        0
    );
    assert!(
        remove_workspace_registration(&registry_path, &workspace_root)
            .expect("remove registration")
    );
    assert!(
        load_workspace_registry(&registry_path)
            .expect("load registry")
            .workspaces
            .is_empty()
    );

    std::fs::remove_dir_all(root).expect("remove temp files");
}

#[test]
fn native_workspace_registry_manages_full_lifecycle() {
    let root = std::env::temp_dir().join(format!(
        "synchub-desktop-native-workspaces-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let first = root.join("alpha");
    let second = root.join("bravo");
    std::fs::create_dir_all(&first).expect("create first workspace");
    std::fs::create_dir_all(&second).expect("create second workspace");
    let registry_path = root.join("desktop-workspaces.json");
    let legacy_config_path = root.join("legacy-config.json");
    let login = CliConfig {
        server_url: "https://sync.example".to_string(),
        user: synchub_desktop::models::User {
            id: "user-1".to_string(),
            email: "user@example.com".to_string(),
            status: "active".to_string(),
        },
        ..CliConfig::default()
    };
    let roots = vec![first.display().to_string(), second.display().to_string()];

    let initialized = initialize_workspaces(
        &roots,
        "/devices",
        &login,
        &registry_path,
        &legacy_config_path,
    )
    .expect("initialize native workspaces");
    assert_eq!(initialized.len(), 2);
    assert_eq!(initialized[0].remote_path, "/devices/alpha");
    assert_eq!(initialized[1].remote_path, "/devices/bravo");
    let first_config: WorkspaceConfig = serde_json::from_str(
        &std::fs::read_to_string(first.join(".synchub").join("workspace.json"))
            .expect("read first workspace config"),
    )
    .expect("decode first workspace config");
    assert_eq!(first_config.server_url, login.server_url);
    assert_eq!(first_config.user_id, login.user.id);
    assert!(first_config.created_at.is_some());

    initialize_workspaces(
        &[first.display().to_string()],
        "/renamed",
        &login,
        &registry_path,
        &legacy_config_path,
    )
    .expect("replace existing registration");
    let registry = load_workspace_registry(&registry_path).expect("load updated registry");
    assert_eq!(registry.workspaces.len(), 2);
    assert_eq!(
        registry
            .workspaces
            .iter()
            .find(|entry| {
                std::fs::canonicalize(&entry.root).expect("resolve registered root")
                    == std::fs::canonicalize(&first).expect("resolve first root")
            })
            .expect("first registration")
            .remote_path,
        "/renamed/alpha"
    );

    assert!(
        remove_workspace_registration(&registry_path, &first).expect("remove first registration")
    );
    assert!(first.join(".synchub").join("workspace.json").is_file());
    std::fs::remove_dir_all(&second).expect("remove second workspace directory");
    assert_eq!(
        prune_workspace_registrations(&registry_path).expect("prune stale registrations"),
        1
    );
    assert!(
        load_workspace_registry(&registry_path)
            .expect("load pruned registry")
            .workspaces
            .is_empty()
    );

    std::fs::remove_dir_all(root).expect("remove temp files");
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

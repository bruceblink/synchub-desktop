use super::commands::{
    run_synchub_cli_daemon, run_synchub_cli_file_download, run_synchub_cli_sync,
    run_synchub_cli_trash_list, run_synchub_cli_trash_restore, run_synchub_cli_workspace_init,
};
use super::time::rfc3339_from_system_time;
use super::{AuthMode, CommandResult, SyncHubDesktop};
use crate::client::{SyncHubClient, refresh_cli_config_if_needed};
use crate::config::{
    load_cli_config, load_workspace_snapshots, remove_cli_config, save_cli_config, save_settings,
};
use crate::models::{
    CliConfig, Device, FileListData, FileNode, FileVersion, SyncConflict, TrashEntry,
    compose_remote_directory_path, conflict_resolution_label, file_version_label,
};
use crate::sync_commands::parse_workspace_paths;
use gpui::*;
use std::path::PathBuf;
use std::time::SystemTime;
impl SyncHubDesktop {
    pub(super) fn reload_local_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cli_config = match load_cli_config(&self.cli_config_path) {
            Ok(config) => config,
            Err(error) => {
                self.message = format!("read login config failed: {error}");
                None
            }
        };
        if let Some(config) = &self.cli_config {
            self.server_input.update(cx, |input, cx| {
                input.set_value(config.server_url.clone(), window, cx);
            });
            self.settings.server_url = config.server_url.clone();
        }
        self.workspaces = match load_workspace_snapshots(&self.registry_path) {
            Ok(workspaces) => workspaces,
            Err(error) => {
                self.message = format!("read workspace registry failed: {error}");
                Vec::new()
            }
        };
        if self.selected_workspace >= self.workspaces.len() {
            self.selected_workspace = 0;
        }
        if let Some(workspace) = self.current_workspace() {
            let root = workspace.root_path().display().to_string();
            self.workspace_input.update(cx, |input, cx| {
                input.set_value(root, window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_local_state(window, cx);
        self.refresh_api(cx);
        self.refresh_files(cx);
        self.refresh_trash(cx);
        self.refresh_devices(cx);
        self.refresh_conflicts(cx);
    }

    pub(super) fn refresh_api(&mut self, cx: &mut Context<Self>) {
        let server = self.current_server(cx);
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let client = SyncHubClient::new(server)?;
                    client.ready().await
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(status) => {
                                this.api_status = Some(if status.status.is_empty() {
                                    "ready".to_string()
                                } else {
                                    status.status
                                });
                                this.message = "API is ready".to_string();
                            }
                            Err(error) => {
                                this.api_status = Some("unreachable".to_string());
                                this.message = format!("API check failed: {error}");
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn authenticate(&mut self, mode: AuthMode, cx: &mut Context<Self>) {
        let server = self.current_server(cx);
        let email = self.email_input.read(cx).value().to_string();
        let password = self.password_input.read(cx).value().to_string();
        let config_path = self.cli_config_path.clone();
        self.auth_mode = mode;
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let client = SyncHubClient::new(&server)?;
                    let data = match mode {
                        AuthMode::Login => client.login(&email, &password).await?,
                        AuthMode::Register => client.register(&email, &password).await?,
                    };
                    let now = SystemTime::now();
                    let cfg = CliConfig {
                        server_url: client.base_url().to_string(),
                        user: data.user,
                        access_token_expires_at: Some(rfc3339_from_system_time(
                            data.tokens.access_token_expires_at(now),
                        )),
                        updated_at: Some(rfc3339_from_system_time(now)),
                        tokens: data.tokens,
                    };
                    save_cli_config(&config_path, &cfg)?;
                    Ok::<CliConfig, anyhow::Error>(cfg)
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(config) => {
                                let email = config.user.email.clone();
                                this.cli_config = Some(config);
                                this.message = format!("signed in as {email}");
                            }
                            Err(error) => this.message = format!("auth failed: {error}"),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn logout(&mut self, cx: &mut Context<Self>) {
        let config = self.cli_config.clone();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    if let Some(config) = &config {
                        if !config.tokens.refresh_token.trim().is_empty() {
                            let client = SyncHubClient::new(&config.server_url)?;
                            let _ = client.logout(&config.tokens.refresh_token).await;
                        }
                    }
                    remove_cli_config(&config_path)
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(()) => {
                                this.cli_config = None;
                                this.files.clear();
                                this.files_next_cursor = None;
                                this.selected_file = None;
                                this.file_versions.clear();
                                this.trash_entries.clear();
                                this.devices.clear();
                                this.conflicts.clear();
                                this.message = "signed out".to_string();
                            }
                            Err(error) => this.message = format!("logout failed: {error}"),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn refresh_files(&mut self, cx: &mut Context<Self>) {
        self.load_files_page(None, false, cx);
    }

    pub(super) fn load_more_files(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = self.files_next_cursor.clone() else {
            self.set_message("no more remote files to load", cx);
            return;
        };
        self.load_files_page(Some(cursor), true, cx);
    }

    fn load_files_page(&mut self, cursor: Option<String>, append: bool, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            return;
        };
        let workspace = self.current_workspace().cloned();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let server = workspace
                        .as_ref()
                        .map(|workspace| workspace.server_url(&config.server_url))
                        .unwrap_or_else(|| config.server_url.clone());
                    let remote_path = workspace
                        .as_ref()
                        .map(|workspace| workspace.remote_path())
                        .unwrap_or_else(|| "/".to_string());
                    let client = SyncHubClient::new(server)?;
                    let data = client
                        .list_files_for_path(
                            &config.tokens.access_token,
                            &remote_path,
                            100,
                            cursor.as_deref(),
                        )
                        .await?;
                    Ok::<(CliConfig, FileListData), anyhow::Error>((config, data))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, data)) => {
                                this.cli_config = Some(config);
                                if append {
                                    this.files.extend(data.items);
                                } else {
                                    this.files = data.items;
                                }
                                this.files_next_cursor = data.next_cursor;
                                this.selected_file = this
                                    .selected_file
                                    .as_ref()
                                    .and_then(|selected| {
                                        this.files.iter().find(|file| file.id == selected.id)
                                    })
                                    .cloned();
                                if this.selected_file.is_none() {
                                    this.file_versions.clear();
                                }
                                let suffix = if this.files_next_cursor.is_some() {
                                    " (more available)"
                                } else {
                                    ""
                                };
                                this.message =
                                    format!("loaded {} remote files{suffix}", this.files.len());
                            }
                            Err(error) => this.message = format!("load files failed: {error}"),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn create_remote_directory(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before creating a remote folder", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before creating a remote folder", cx);
            return;
        };
        let input = self.remote_directory_input.read(cx).value().to_string();
        let Some(remote_path) = compose_remote_directory_path(&input, &workspace.remote_path())
        else {
            self.set_message("remote folder path is invalid", cx);
            return;
        };
        let device_id = workspace.device_id();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let server = workspace.server_url(&config.server_url);
                    let client = SyncHubClient::new(server)?;
                    let node = client
                        .create_directory(
                            &config.tokens.access_token,
                            &remote_path,
                            Some(device_id.as_str()),
                        )
                        .await?;
                    let files = client
                        .list_files_for_path(
                            &config.tokens.access_token,
                            &workspace.remote_path(),
                            100,
                            None,
                        )
                        .await?;
                    Ok::<(CliConfig, FileNode, Vec<FileNode>), anyhow::Error>((
                        config,
                        node,
                        files.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, node, files)) => {
                                this.cli_config = Some(config);
                                this.files = files;
                                this.files_next_cursor = None;
                                this.message = format!("created remote folder {}", node.path);
                            }
                            Err(error) => {
                                this.message = format!("create remote folder failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn show_file_versions(&mut self, file: FileNode, cx: &mut Context<Self>) {
        if file.node_type != "file" {
            self.set_message("only remote files have version history", cx);
            return;
        }
        self.selected_file = Some(file.clone());
        self.file_versions.clear();
        self.active_view = super::MainView::Versions;
        self.refresh_file_versions(file, cx);
    }

    pub(super) fn refresh_selected_file_versions(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.selected_file.clone() else {
            self.set_message("select a remote file before loading versions", cx);
            return;
        };
        self.refresh_file_versions(file, cx);
    }

    fn refresh_file_versions(&mut self, file: FileNode, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before loading file versions", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = self
            .current_workspace()
            .map(|workspace| workspace.server_url(&config.server_url))
            .unwrap_or_else(|| config.server_url.clone());
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let client = SyncHubClient::new(server)?;
                    let data = client
                        .list_file_versions(&config.tokens.access_token, &file.id, 100)
                        .await?;
                    Ok::<(CliConfig, FileNode, Vec<FileVersion>), anyhow::Error>((
                        config, file, data.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, file, versions)) => {
                                this.cli_config = Some(config);
                                this.selected_file = Some(file.clone());
                                this.file_versions = versions;
                                this.message = format!(
                                    "loaded {} version(s) for {}",
                                    this.file_versions.len(),
                                    file.path
                                );
                            }
                            Err(error) => {
                                this.message = format!("load file versions failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn restore_file_version(&mut self, version: FileVersion, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before restoring a file version", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before restoring a file version", cx);
            return;
        };
        let Some(file) = self.selected_file.clone() else {
            self.set_message("select a remote file before restoring a version", cx);
            return;
        };
        let device_id = workspace.device_id();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result =
                    async {
                        let changed = refresh_cli_config_if_needed(&mut config).await?;
                        if changed {
                            save_cli_config(&config_path, &config)?;
                        }
                        let server = workspace.server_url(&config.server_url);
                        let client = SyncHubClient::new(server)?;
                        let restored = client
                            .restore_file_version(
                                &config.tokens.access_token,
                                &file.id,
                                version.version,
                                Some(device_id.as_str()),
                            )
                            .await?;
                        let versions = client
                            .list_file_versions(&config.tokens.access_token, &file.id, 100)
                            .await?;
                        let files = client
                            .list_files_for_path(
                                &config.tokens.access_token,
                                &workspace.remote_path(),
                                100,
                                None,
                            )
                            .await?;
                        Ok::<(CliConfig, FileNode, Vec<FileVersion>, Vec<FileNode>), anyhow::Error>(
                            (config, restored.file, versions.items, files.items),
                        )
                    }
                    .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, restored, versions, files)) => {
                                this.cli_config = Some(config);
                                this.selected_file = Some(restored.clone());
                                this.file_versions = versions;
                                this.files = files;
                                this.files_next_cursor = None;
                                this.message = format!(
                                    "restored {} to {}",
                                    restored.path,
                                    file_version_label(&version)
                                );
                            }
                            Err(error) => {
                                this.message = format!("restore file version failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn set_file_version_pin(
        &mut self,
        version: FileVersion,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before updating a file version", cx);
            return;
        };
        let Some(file) = self.selected_file.clone() else {
            self.set_message("select a remote file before updating a version", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = self
            .current_workspace()
            .map(|workspace| workspace.server_url(&config.server_url))
            .unwrap_or_else(|| config.server_url.clone());
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let client = SyncHubClient::new(server)?;
                    let updated = if pinned {
                        client
                            .pin_file_version(
                                &config.tokens.access_token,
                                &file.id,
                                version.version,
                            )
                            .await?
                    } else {
                        client
                            .unpin_file_version(
                                &config.tokens.access_token,
                                &file.id,
                                version.version,
                            )
                            .await?
                    };
                    let versions = client
                        .list_file_versions(&config.tokens.access_token, &file.id, 100)
                        .await?;
                    Ok::<(CliConfig, FileVersion, Vec<FileVersion>), anyhow::Error>((
                        config,
                        updated,
                        versions.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, updated, versions)) => {
                                this.cli_config = Some(config);
                                this.file_versions = versions;
                                this.message = format!(
                                    "{} {}",
                                    if pinned { "pinned" } else { "unpinned" },
                                    file_version_label(&updated)
                                );
                            }
                            Err(error) => {
                                this.message = format!("update file version failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn delete_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before deleting a remote item", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before deleting a remote item", cx);
            return;
        };
        let device_id = workspace.device_id();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let server = workspace.server_url(&config.server_url);
                    let client = SyncHubClient::new(server)?;
                    client
                        .delete_file(
                            &config.tokens.access_token,
                            &file.id,
                            Some(device_id.as_str()),
                        )
                        .await?;
                    let files = client
                        .list_files_for_path(
                            &config.tokens.access_token,
                            &workspace.remote_path(),
                            100,
                            None,
                        )
                        .await?;
                    Ok::<(CliConfig, FileNode, Vec<FileNode>), anyhow::Error>((
                        config,
                        file,
                        files.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, file, files)) => {
                                this.cli_config = Some(config);
                                this.files = files;
                                this.files_next_cursor = None;
                                if this
                                    .selected_file
                                    .as_ref()
                                    .map(|selected| selected.id == file.id)
                                    .unwrap_or(false)
                                {
                                    this.selected_file = None;
                                    this.file_versions.clear();
                                }
                                this.message = format!("deleted remote item {}", file.path);
                            }
                            Err(error) => {
                                this.message = format!("delete remote item failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn move_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before moving a remote item", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before moving a remote item", cx);
            return;
        };
        let input = self.remote_target_input.read(cx).value().to_string();
        let Some(target_path) = compose_remote_directory_path(&input, &workspace.remote_path())
        else {
            self.set_message("move target path is invalid", cx);
            return;
        };
        let device_id = workspace.device_id();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let server = workspace.server_url(&config.server_url);
                    let client = SyncHubClient::new(server)?;
                    let moved = client
                        .move_file(
                            &config.tokens.access_token,
                            &file.id,
                            &target_path,
                            Some(device_id.as_str()),
                        )
                        .await?;
                    let files = client
                        .list_files_for_path(
                            &config.tokens.access_token,
                            &workspace.remote_path(),
                            100,
                            None,
                        )
                        .await?;
                    Ok::<(CliConfig, FileNode, FileNode, Vec<FileNode>), anyhow::Error>((
                        config,
                        file,
                        moved,
                        files.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, old, moved, files)) => {
                                this.cli_config = Some(config);
                                this.files = files;
                                this.files_next_cursor = None;
                                if this
                                    .selected_file
                                    .as_ref()
                                    .map(|selected| selected.id == moved.id)
                                    .unwrap_or(false)
                                {
                                    this.selected_file = Some(moved.clone());
                                }
                                this.message =
                                    format!("moved remote item {} -> {}", old.path, moved.path);
                            }
                            Err(error) => {
                                this.message = format!("move remote item failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn download_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before downloading a remote file", cx);
            return;
        };
        if file.node_type != "file" {
            self.set_message("only remote files can be downloaded", cx);
            return;
        }
        let workspace_root = workspace.root_path();
        let workspace_config = workspace.workspace_config_path();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_file_download(
                        &workspace_root,
                        &workspace_config,
                        &config_path,
                        &file,
                    )
                })
                .await
                .unwrap_or_else(|error| CommandResult {
                    ok: false,
                    summary: format!("download failed: {error}"),
                    output: String::new(),
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn refresh_trash(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before loading local trash", cx);
            return;
        };
        let workspace_root = workspace.root_path();
        let workspace_config = workspace.workspace_config_path();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let (result, entries) = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_trash_list(&workspace_root, &workspace_config)
                })
                .await
                .unwrap_or_else(|error| {
                    (
                        CommandResult {
                            ok: false,
                            summary: format!("load trash failed: {error}"),
                            output: String::new(),
                        },
                        Vec::new(),
                    )
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if result.ok {
                            this.trash_entries = entries;
                        }
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn restore_trash_entry(&mut self, entry: TrashEntry, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before restoring trash", cx);
            return;
        };
        let workspace_root = workspace.root_path();
        let workspace_config = workspace.workspace_config_path();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let (result, entries) = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_trash_restore(&workspace_root, &workspace_config, &entry)
                })
                .await
                .unwrap_or_else(|error| {
                    (
                        CommandResult {
                            ok: false,
                            summary: format!("restore trash failed: {error}"),
                            output: String::new(),
                        },
                        None,
                    )
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if let Some(entries) = entries {
                            this.trash_entries = entries;
                        }
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn refresh_conflicts(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            return;
        };
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let client = SyncHubClient::new(&config.server_url)?;
                    let data = client
                        .list_conflicts(&config.tokens.access_token, 100)
                        .await?;
                    Ok::<(CliConfig, Vec<SyncConflict>), anyhow::Error>((config, data.items))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, conflicts)) => {
                                this.cli_config = Some(config);
                                this.conflicts = conflicts;
                                this.message =
                                    format!("loaded {} pending conflicts", this.conflicts.len());
                            }
                            Err(error) => this.message = format!("load conflicts failed: {error}"),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = self
            .current_workspace()
            .map(|workspace| workspace.server_url(&config.server_url))
            .unwrap_or_else(|| config.server_url.clone());
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let client = SyncHubClient::new(server)?;
                    let data = client
                        .list_devices(&config.tokens.access_token, 100)
                        .await?;
                    Ok::<(CliConfig, Vec<Device>), anyhow::Error>((config, data.items))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, devices)) => {
                                this.cli_config = Some(config);
                                this.devices = devices;
                                this.message = format!("loaded {} devices", this.devices.len());
                            }
                            Err(error) => this.message = format!("load devices failed: {error}"),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn resolve_conflict(
        &mut self,
        conflict_id: String,
        resolution: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before resolving conflicts", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let client = SyncHubClient::new(&config.server_url)?;
                    let resolved = client
                        .resolve_conflict(&config.tokens.access_token, &conflict_id, resolution)
                        .await?;
                    let conflicts = client
                        .list_conflicts(&config.tokens.access_token, 100)
                        .await?;
                    Ok::<(CliConfig, SyncConflict, Vec<SyncConflict>), anyhow::Error>((
                        config,
                        resolved,
                        conflicts.items,
                    ))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, resolved, conflicts)) => {
                                this.cli_config = Some(config);
                                this.conflicts = conflicts;
                                this.message = format!(
                                    "resolved {} as {}",
                                    resolved.path,
                                    conflict_resolution_label(resolution)
                                );
                            }
                            Err(error) => {
                                this.message = format!("resolve conflict failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn run_sync_command(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let workspace_root = self
            .current_workspace()
            .map(|workspace| workspace.root_path())
            .unwrap_or_else(|| PathBuf::from(self.workspace_input.read(cx).value().as_ref()));
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_sync(action, &workspace_root, &config_path)
                })
                .await
                .unwrap_or_else(|error| CommandResult {
                    ok: false,
                    summary: format!("sync command failed: {error}"),
                    output: String::new(),
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn run_daemon_command(&mut self, action: &str, cx: &mut Context<Self>) {
        let workspace_root = self
            .current_workspace()
            .map(|workspace| workspace.root_path())
            .unwrap_or_else(|| PathBuf::from(self.workspace_input.read(cx).value().as_ref()));
        let config_path = self.cli_config_path.clone();
        let action = action.to_string();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_daemon(&action, &workspace_root, &config_path)
                })
                .await
                .unwrap_or_else(|error| CommandResult {
                    ok: false,
                    summary: format!("daemon command failed: {error}"),
                    output: String::new(),
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn init_workspace(&mut self, cx: &mut Context<Self>) {
        let roots = parse_workspace_paths(self.workspace_input.read(cx).value().as_ref());
        if roots.is_empty() {
            self.set_message("workspace path is required", cx);
            return;
        }
        let remote_root = self.remote_root_input.read(cx).value().to_string();
        let config_path = self.cli_config_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    run_synchub_cli_workspace_init(&roots, &remote_root, &config_path)
                })
                .await
                .unwrap_or_else(|error| CommandResult {
                    ok: false,
                    summary: format!("workspace init failed: {error}"),
                    output: String::new(),
                });
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        this.command_result = Some(result.clone());
                        this.message = result.summary.clone();
                        if let Ok(workspaces) = load_workspace_snapshots(&this.registry_path) {
                            this.workspaces = workspaces;
                            this.selected_workspace = this.workspaces.len().saturating_sub(1);
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn save_server(&mut self, cx: &mut Context<Self>) {
        self.settings.server_url = self.current_server(cx);
        match save_settings(&self.settings) {
            Ok(()) => self.set_message("server saved", cx),
            Err(error) => self.set_message(format!("save settings failed: {error}"), cx),
        }
    }
}

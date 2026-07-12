use super::time::rfc3339_from_system_time;
use super::{AuthMode, CommandResult, SyncHubDesktop};
use crate::client::{SyncHubClient, normalize_base_url, refresh_cli_config_if_needed};
use crate::config::{
    initialize_workspaces, load_cli_config, load_workspace_snapshots,
    prune_workspace_registrations, remove_cli_config, remove_workspace_registration,
    save_cli_config, save_settings, update_cli_server_url, update_workspace_server_urls,
};
use crate::models::{
    CliConfig, Device, FileListData, FileNode, FileVersion, SyncConflict, TrashEntry,
    compose_remote_directory_path, conflict_resolution_label, file_belongs_to_remote_root,
    file_version_label,
};
use crate::native_daemon::{reset_state as reset_daemon_state, set_paused, start_daemon};
use crate::native_doctor::run_doctor;
use crate::native_download::{local_path_for_remote, write_downloaded_file};
use crate::native_manifest::scan_and_save_manifest;
use crate::native_sync::{build_sync_plan, execute_pull, execute_push, execute_sync_once};
use crate::native_trash::{list_trash_entries, restore_trash_entry as restore_local_trash_entry};
use crate::sync_commands::parse_workspace_paths;
use gpui::*;
use std::time::SystemTime;
impl SyncHubDesktop {
    pub(super) fn select_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        if index == self.selected_workspace {
            return;
        }
        self.selected_workspace = index;
        self.files.clear();
        self.files_next_cursor = None;
        self.selected_file = None;
        self.file_versions.clear();
        self.trash_entries.clear();
        self.cloud_trash.clear();
        self.devices.clear();
        self.conflicts.clear();
        self.active_view = super::MainView::Overview;
        cx.notify();
    }

    pub(super) fn reload_local_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cli_config = match load_cli_config(&self.cli_config_path) {
            Ok(config) => config,
            Err(error) => {
                self.message = format!("read login config failed: {error}");
                None
            }
        };
        let server_url = normalize_base_url(&self.settings.server_url);
        self.settings.server_url = server_url.clone();
        self.server_input.update(cx, |input, cx| {
            input.set_value(server_url.clone(), window, cx);
        });
        if self.cli_config.is_some() {
            match update_cli_server_url(&self.cli_config_path, &server_url) {
                Ok(config) => self.cli_config = config,
                Err(error) => self.message = format!("update login server failed: {error}"),
            }
        }
        if let Err(error) = update_workspace_server_urls(&self.registry_path, &server_url) {
            self.message = format!("update workspace server failed: {error}");
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
        self.refresh_server_status(cx);
        self.refresh_files(cx);
        self.refresh_trash(cx);
        self.refresh_cloud_trash(cx);
        self.refresh_devices(cx);
        self.refresh_conflicts(cx);
    }

    pub(super) fn refresh_api(&mut self, cx: &mut Context<Self>) {
        self.refresh_server_status(cx);
    }

    pub(super) fn refresh_server_status(&mut self, cx: &mut Context<Self>) {
        let server = self.current_server(cx);
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let client = SyncHubClient::new(server)?;
                    let version = client.version().await?;
                    let health = client.health().await?;
                    let ready = client.ready().await?;
                    Ok::<_, anyhow::Error>((version, health, ready))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((version, health, ready)) => {
                                this.api_status = Some(if ready.status.is_empty() {
                                    "ready".to_string()
                                } else {
                                    ready.status.clone()
                                });
                                this.server_version = Some(version);
                                this.server_health = Some(health);
                                this.server_ready = Some(ready);
                                this.message = "server status loaded".to_string();
                            }
                            Err(error) => {
                                this.api_status = Some("unreachable".to_string());
                                this.server_version = None;
                                this.server_health = None;
                                this.server_ready = None;
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

    pub(super) fn refresh_server_metrics(&mut self, cx: &mut Context<Self>) {
        let server = self.current_server(cx);
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let client = SyncHubClient::new(server)?;
                    client.metrics().await
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(metrics) => {
                                let bytes = metrics.len();
                                this.server_result = Some(CommandResult {
                                    ok: true,
                                    summary: format!("server metrics loaded ({bytes} bytes)"),
                                    output: metrics,
                                });
                            }
                            Err(error) => {
                                this.server_result = Some(CommandResult {
                                    ok: false,
                                    summary: format!("server metrics failed: {error}"),
                                    output: String::new(),
                                });
                            }
                        }
                        if let Some(result) = &this.server_result {
                            this.message = result.summary.clone();
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn refresh_server_openapi(&mut self, cx: &mut Context<Self>) {
        let server = self.current_server(cx);
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let client = SyncHubClient::new(server)?;
                    client.openapi().await
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(spec) => {
                                let bytes = spec.len();
                                this.server_result = Some(CommandResult {
                                    ok: true,
                                    summary: format!("OpenAPI spec loaded ({bytes} bytes)"),
                                    output: spec,
                                });
                            }
                            Err(error) => {
                                this.server_result = Some(CommandResult {
                                    ok: false,
                                    summary: format!("OpenAPI load failed: {error}"),
                                    output: String::new(),
                                });
                            }
                        }
                        if let Some(result) = &this.server_result {
                            this.message = result.summary.clone();
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
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before downloading a remote file", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before downloading a remote file", cx);
            return;
        };
        if file.node_type != "file" {
            self.set_message("only remote files can be downloaded", cx);
            return;
        }
        let workspace_root = workspace.root_path();
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
        let remote_root = workspace.remote_path();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = async {
                    let changed = refresh_cli_config_if_needed(&mut config).await?;
                    if changed {
                        save_cli_config(&config_path, &config)?;
                    }
                    let target = local_path_for_remote(&workspace_root, &remote_root, &file.path)?;
                    let content = SyncHubClient::new(server)?
                        .download_file(&config.tokens.access_token, &file.id)
                        .await?;
                    let (target, bytes) = tokio::task::spawn_blocking(move || {
                        write_downloaded_file(&target, &content).map(|bytes| (target, bytes))
                    })
                    .await??;
                    Ok::<_, anyhow::Error>((config, target, bytes))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, target, bytes)) => {
                                this.cli_config = Some(config);
                                this.message =
                                    format!("downloaded {bytes} bytes to {}", target.display());
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
                            Err(error) => {
                                this.message = format!("download failed: {error}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
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

    pub(super) fn refresh_trash(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before loading local trash", cx);
            return;
        };
        let workspace_root = workspace.root_path();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result =
                    tokio::task::spawn_blocking(move || list_trash_entries(&workspace_root, 200))
                        .await
                        .map_err(|error| anyhow::anyhow!("load trash task failed: {error}"))
                        .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(entries) => {
                                this.message = format!("loaded {} trash item(s)", entries.len());
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                                this.trash_entries = entries;
                            }
                            Err(error) => {
                                this.message = format!("load trash failed: {error}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
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

    pub(super) fn refresh_cloud_trash(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before loading cloud trash", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before loading cloud trash", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
        let remote_root = workspace.remote_path();
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
                    let trash = client
                        .list_trash(&config.tokens.access_token)
                        .await?
                        .into_iter()
                        .filter(|file| file_belongs_to_remote_root(&file.path, &remote_root))
                        .collect::<Vec<_>>();
                    Ok::<_, anyhow::Error>((config, trash))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, trash)) => {
                                this.cli_config = Some(config);
                                this.cloud_trash = trash;
                                this.message =
                                    format!("loaded {} cloud trash items", this.cloud_trash.len());
                            }
                            Err(error) => {
                                this.message = format!("load cloud trash failed: {error}")
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn restore_cloud_trash(&mut self, file: FileNode, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before restoring cloud trash", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before restoring cloud trash", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
        let remote_root = workspace.remote_path();
        let device_id = workspace.device_id();
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
                    let restored = client
                        .restore_trash(
                            &config.tokens.access_token,
                            &file.id,
                            Some(device_id.as_str()),
                        )
                        .await?;
                    let trash = client
                        .list_trash(&config.tokens.access_token)
                        .await?
                        .into_iter()
                        .filter(|item| file_belongs_to_remote_root(&item.path, &remote_root))
                        .collect::<Vec<_>>();
                    Ok::<_, anyhow::Error>((config, restored, trash))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, restored, trash)) => {
                                this.cli_config = Some(config);
                                this.cloud_trash = trash;
                                this.message =
                                    format!("restored {} from cloud trash", restored.path);
                            }
                            Err(error) => {
                                this.message = format!("restore cloud trash failed: {error}")
                            }
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
        let restored_entry_path = entry.path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    restore_local_trash_entry(&workspace_root, &entry)?;
                    list_trash_entries(&workspace_root, 200)
                })
                .await
                .map_err(|error| anyhow::anyhow!("restore trash task failed: {error}"))
                .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok(entries) => {
                                this.message = format!("restored trash item {restored_entry_path}");
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                                this.trash_entries = entries;
                            }
                            Err(error) => {
                                this.message = format!("restore trash failed: {error}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
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
        if matches!(action, "status" | "dry-run") {
            self.run_native_sync_preview(action, cx);
            return;
        }
        if action == "push" {
            self.run_native_push(cx);
            return;
        }
        if action == "pull" {
            self.run_native_pull(cx);
            return;
        }
        if action == "once" {
            self.run_native_sync_once(cx);
            return;
        }
        if action == "doctor" {
            self.run_native_doctor(cx);
            return;
        }
        self.set_message(format!("unsupported sync action: {action}"), cx);
    }

    fn run_native_doctor(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before running diagnostics", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before running diagnostics", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
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
                    let report = run_doctor(&client, &config, &workspace).await?;
                    Ok::<_, anyhow::Error>((config, report))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, report)) => {
                                this.cli_config = Some(config);
                                this.message = report.summary();
                                this.command_result = Some(CommandResult {
                                    ok: report.ok(),
                                    summary: this.message.clone(),
                                    output: report.display(),
                                });
                            }
                            Err(error) => {
                                this.message = format!("doctor failed: {error:#}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn run_native_push(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before pushing local changes", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before pushing local changes", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
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
                    let pushed =
                        execute_push(&client, &config.tokens.access_token, &workspace).await?;
                    Ok::<_, anyhow::Error>((config, pushed))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, pushed)) => {
                                this.cli_config = Some(config);
                                this.message = pushed.summary();
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                                if let Ok(workspaces) =
                                    load_workspace_snapshots(&this.registry_path)
                                {
                                    this.workspaces = workspaces;
                                }
                            }
                            Err(error) => {
                                this.message = format!("push failed: {error:#}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn run_native_pull(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before pulling remote changes", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before pulling remote changes", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
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
                    let pulled =
                        execute_pull(&client, &config.tokens.access_token, &workspace).await?;
                    Ok::<_, anyhow::Error>((config, pulled))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, pulled)) => {
                                this.cli_config = Some(config);
                                this.message = pulled.summary();
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: format!(
                                        "cursor: {}\ntrashed: {}",
                                        pulled.cursor, pulled.trashed
                                    ),
                                });
                                if let Ok(workspaces) =
                                    load_workspace_snapshots(&this.registry_path)
                                {
                                    this.workspaces = workspaces;
                                }
                            }
                            Err(error) => {
                                this.message = format!("pull failed: {error:#}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn run_native_sync_once(&mut self, cx: &mut Context<Self>) {
        let Some(mut config) = self.cli_config.clone() else {
            self.set_message("sign in before syncing", cx);
            return;
        };
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before syncing", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let server = workspace.server_url(&config.server_url);
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
                    let synced =
                        execute_sync_once(&client, &config.tokens.access_token, &workspace).await?;
                    Ok::<_, anyhow::Error>((config, synced))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, synced)) => {
                                this.cli_config = Some(config);
                                this.message = synced.summary();
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: format!("cursor: {}", synced.pull.cursor),
                                });
                            }
                            Err(error) => {
                                this.message = format!("sync failed: {error:#}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
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

    fn run_native_sync_preview(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before checking sync status", cx);
            return;
        };
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || build_sync_plan(&workspace))
                    .await
                    .map_err(|error| anyhow::anyhow!("sync preview task failed: {error}"))
                    .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((_, plan)) => {
                                let prefix = if action == "dry-run" {
                                    "Dry run"
                                } else {
                                    "Sync status"
                                };
                                this.message = format!("{prefix}: {}", plan.summary());
                                this.command_result = Some(CommandResult {
                                    ok: true,
                                    summary: this.message.clone(),
                                    output: plan.display(),
                                });
                            }
                            Err(error) => {
                                this.message = format!("sync preview failed: {error}");
                                this.command_result = Some(CommandResult {
                                    ok: false,
                                    summary: this.message.clone(),
                                    output: String::new(),
                                });
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn run_daemon_command(&mut self, action: &str, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before managing background sync", cx);
            return;
        };
        let root = workspace.root_path();
        let key = root.display().to_string();
        let result = match action {
            "start" => {
                let running = self
                    .daemon_tasks
                    .get(&key)
                    .is_some_and(|task| !task.is_finished());
                if running {
                    Ok("Background sync is already running".to_string())
                } else {
                    set_paused(&root, false).map(|_| {
                        let task = start_daemon(workspace.entry, self.cli_config_path.clone());
                        self.daemon_tasks.insert(key, task);
                        "Background sync started".to_string()
                    })
                }
            }
            "pause" => set_paused(&root, true).map(|_| "Background sync paused".to_string()),
            "resume" => set_paused(&root, false).map(|_| "Background sync resumed".to_string()),
            "reset-state" => {
                reset_daemon_state(&root).map(|_| "Background sync state reset".to_string())
            }
            "status" => Ok(if self
                .daemon_tasks
                .get(&key)
                .is_some_and(|task| !task.is_finished())
            {
                "Background sync is running"
            } else {
                "Background sync is not running"
            }
            .to_string()),
            _ => Err(anyhow::anyhow!("unknown background sync action: {action}")),
        };
        let command_result = match result {
            Ok(summary) => CommandResult {
                ok: true,
                summary,
                output: String::new(),
            },
            Err(error) => CommandResult {
                ok: false,
                summary: format!("background sync failed: {error:#}"),
                output: String::new(),
            },
        };
        self.message = command_result.summary.clone();
        self.command_result = Some(command_result);
        if let Ok(workspaces) = load_workspace_snapshots(&self.registry_path) {
            self.workspaces = workspaces;
        }
        cx.notify();
    }

    pub(super) fn init_workspace(&mut self, cx: &mut Context<Self>) {
        let roots = parse_workspace_paths(self.workspace_input.read(cx).value().as_ref());
        if roots.is_empty() {
            self.set_message("workspace path is required", cx);
            return;
        }
        let remote_root = self.remote_root_input.read(cx).value().to_string();
        let Some(login) = self.cli_config.clone() else {
            self.set_message("sign in before initializing a workspace", cx);
            return;
        };
        let config_path = self.cli_config_path.clone();
        let registry_path = self.registry_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    initialize_workspaces(
                        &roots,
                        &remote_root,
                        &login,
                        &registry_path,
                        &config_path,
                    )
                })
                .await
                .map_err(|error| anyhow::anyhow!("workspace init task failed: {error}"))
                .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        let command_result = match result {
                            Ok(entries) => CommandResult {
                                ok: true,
                                summary: format!("initialized {} workspace(s)", entries.len()),
                                output: String::new(),
                            },
                            Err(error) => CommandResult {
                                ok: false,
                                summary: format!("workspace init failed: {error}"),
                                output: String::new(),
                            },
                        };
                        this.message = command_result.summary.clone();
                        if command_result.ok
                            && let Ok(workspaces) = load_workspace_snapshots(&this.registry_path)
                        {
                            this.workspaces = workspaces;
                            this.selected_workspace = this.workspaces.len().saturating_sub(1);
                        }
                        this.command_result = Some(command_result);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn remove_selected_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before removing it", cx);
            return;
        };
        let workspace_root = workspace.root_path();
        let registry_path = self.registry_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    remove_workspace_registration(&registry_path, &workspace_root)
                })
                .await
                .map_err(|error| anyhow::anyhow!("workspace remove task failed: {error}"))
                .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        let command_result = match result {
                            Ok(true) => CommandResult {
                                ok: true,
                                summary: "workspace registration removed".to_string(),
                                output: String::new(),
                            },
                            Ok(false) => CommandResult {
                                ok: false,
                                summary: "workspace registration not found".to_string(),
                                output: String::new(),
                            },
                            Err(error) => CommandResult {
                                ok: false,
                                summary: format!("workspace remove failed: {error}"),
                                output: String::new(),
                            },
                        };
                        this.message = command_result.summary.clone();
                        if command_result.ok {
                            this.reload_workspace_list_after_registry_change();
                        }
                        this.command_result = Some(command_result);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn prune_workspaces(&mut self, cx: &mut Context<Self>) {
        let registry_path = self.registry_path.clone();
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    prune_workspace_registrations(&registry_path)
                })
                .await
                .map_err(|error| anyhow::anyhow!("workspace prune task failed: {error}"))
                .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        let command_result = match result {
                            Ok(removed) => CommandResult {
                                ok: true,
                                summary: format!(
                                    "pruned {removed} stale workspace registration(s)"
                                ),
                                output: String::new(),
                            },
                            Err(error) => CommandResult {
                                ok: false,
                                summary: format!("workspace prune failed: {error}"),
                                output: String::new(),
                            },
                        };
                        this.message = command_result.summary.clone();
                        if command_result.ok {
                            this.reload_workspace_list_after_registry_change();
                        }
                        this.command_result = Some(command_result);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn reload_workspace_list_after_registry_change(&mut self) {
        if let Ok(workspaces) = load_workspace_snapshots(&self.registry_path) {
            let previous = self.selected_workspace;
            self.workspaces = workspaces;
            if self.selected_workspace >= self.workspaces.len() {
                self.selected_workspace = self.workspaces.len().saturating_sub(1);
            }
            if previous != self.selected_workspace || self.workspaces.is_empty() {
                self.files.clear();
                self.files_next_cursor = None;
                self.selected_file = None;
                self.file_versions.clear();
                self.trash_entries.clear();
                self.cloud_trash.clear();
            }
        }
    }

    pub(super) fn scan_selected_manifest(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.current_workspace().cloned() else {
            self.set_message("select a workspace before scanning manifest", cx);
            return;
        };
        self.set_loading(true, cx);
        cx.spawn(move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result =
                    tokio::task::spawn_blocking(move || scan_and_save_manifest(&workspace))
                        .await
                        .map_err(|error| anyhow::anyhow!("manifest scan task failed: {error}"))
                        .and_then(|result| result);
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        let command_result = match result {
                            Ok(manifest) => CommandResult {
                                ok: true,
                                summary: format!("scanned {} local file(s)", manifest.items.len()),
                                output: String::new(),
                            },
                            Err(error) => CommandResult {
                                ok: false,
                                summary: format!("manifest scan failed: {error}"),
                                output: String::new(),
                            },
                        };
                        this.message = command_result.summary.clone();
                        if command_result.ok {
                            this.reload_workspace_list_after_registry_change();
                        }
                        this.command_result = Some(command_result);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn save_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let server_url = normalize_base_url(&self.current_server(cx));
        self.settings.server_url = server_url.clone();
        let result = (|| {
            save_settings(&self.settings)?;
            self.cli_config = update_cli_server_url(&self.cli_config_path, &server_url)?;
            update_workspace_server_urls(&self.registry_path, &server_url)?;
            self.workspaces = load_workspace_snapshots(&self.registry_path)?;
            Ok::<(), anyhow::Error>(())
        })();
        match result {
            Ok(()) => {
                self.server_input.update(cx, |input, cx| {
                    input.set_value(server_url, window, cx);
                });
                self.set_message("server saved to local configuration", cx);
            }
            Err(error) => self.set_message(format!("save server failed: {error}"), cx),
        }
    }
}

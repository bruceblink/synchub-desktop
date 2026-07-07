use crate::client::{SyncHubClient, refresh_cli_config_if_needed};
use crate::config::{
    DesktopSettings, default_cli_config_path, default_workspace_registry_path, load_cli_config,
    load_settings, load_workspace_snapshots, remove_cli_config, save_cli_config, save_settings,
};
use crate::models::{
    CliConfig, Device, FileNode, SyncConflict, SyncTrashSnapshot, TrashEntry, WorkspaceSnapshot,
    compose_remote_directory_path, conflict_resolution_label, format_bytes, is_current_device,
    workspace_metrics,
};
use crate::sync_commands::{
    file_download_command_args, parse_workspace_paths, sync_action_label, sync_command_args,
    trash_list_command_args, trash_restore_command_args, workspace_init_command_args,
};
use crate::theme::{ThemeColors, alpha};
use gpui::prelude::*;
use gpui::*;
use gpui_component::{
    Icon, IconName, TitleBar,
    button::*,
    input::{Input, InputState},
    label::Label,
    scroll::ScrollableElement,
    *,
};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainView {
    Overview,
    Sync,
    Files,
    Trash,
    Devices,
    Conflicts,
    Daemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthMode {
    Login,
    Register,
}

#[derive(Clone, Debug, Default)]
struct CommandResult {
    ok: bool,
    summary: String,
    output: String,
}

pub struct SyncHubDesktop {
    server_input: Entity<InputState>,
    email_input: Entity<InputState>,
    password_input: Entity<InputState>,
    workspace_input: Entity<InputState>,
    remote_root_input: Entity<InputState>,
    remote_directory_input: Entity<InputState>,
    remote_target_input: Entity<InputState>,
    settings: DesktopSettings,
    cli_config_path: PathBuf,
    registry_path: PathBuf,
    cli_config: Option<CliConfig>,
    workspaces: Vec<WorkspaceSnapshot>,
    selected_workspace: usize,
    api_status: Option<String>,
    files: Vec<FileNode>,
    trash_entries: Vec<TrashEntry>,
    devices: Vec<Device>,
    conflicts: Vec<SyncConflict>,
    active_view: MainView,
    auth_mode: AuthMode,
    loading: bool,
    message: String,
    command_result: Option<CommandResult>,
    colors: ThemeColors,
}

impl SyncHubDesktop {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = load_settings();
        let server_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Server URL")
                .default_value(settings.server_url.clone())
        });
        let email_input = cx.new(|cx| InputState::new(window, cx).placeholder("Email"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let workspace_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Workspace paths"));
        let remote_root_input = cx.new(|cx| InputState::new(window, cx).placeholder("Remote root"));
        let remote_directory_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("New remote folder"));
        let remote_target_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Move target path"));

        let cli_config_path = default_cli_config_path();
        let registry_path = default_workspace_registry_path(&cli_config_path);
        let mut app = Self {
            server_input,
            email_input,
            password_input,
            workspace_input,
            remote_root_input,
            remote_directory_input,
            remote_target_input,
            settings,
            cli_config_path,
            registry_path,
            cli_config: None,
            workspaces: Vec::new(),
            selected_workspace: 0,
            api_status: None,
            files: Vec::new(),
            trash_entries: Vec::new(),
            devices: Vec::new(),
            conflicts: Vec::new(),
            active_view: MainView::Overview,
            auth_mode: AuthMode::Login,
            loading: false,
            message: String::new(),
            command_result: None,
            colors: ThemeColors::default(),
        };
        app.reload_local_state(window, cx);
        app
    }

    fn current_server(&self, cx: &App) -> String {
        self.server_input.read(cx).value().to_string()
    }

    fn current_workspace(&self) -> Option<&WorkspaceSnapshot> {
        self.workspaces.get(self.selected_workspace)
    }

    fn set_message(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = message.into();
        cx.notify();
    }

    fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }

    fn reload_local_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_local_state(window, cx);
        self.refresh_api(cx);
        self.refresh_files(cx);
        self.refresh_trash(cx);
        self.refresh_devices(cx);
        self.refresh_conflicts(cx);
    }

    fn refresh_api(&mut self, cx: &mut Context<Self>) {
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

    fn authenticate(&mut self, mode: AuthMode, cx: &mut Context<Self>) {
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

    fn logout(&mut self, cx: &mut Context<Self>) {
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

    fn refresh_files(&mut self, cx: &mut Context<Self>) {
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
                    let data = client.list_files(&config.tokens.access_token, 100).await?;
                    Ok::<(CliConfig, Vec<FileNode>), anyhow::Error>((config, data.items))
                }
                .await;
                if let Some(this) = this.upgrade() {
                    let _ = this.update(&mut cx, |this, cx| {
                        this.loading = false;
                        match result {
                            Ok((config, files)) => {
                                this.cli_config = Some(config);
                                this.files = files;
                                this.message = format!("loaded {} remote files", this.files.len());
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

    fn create_remote_directory(&mut self, cx: &mut Context<Self>) {
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
                    let files = client.list_files(&config.tokens.access_token, 100).await?;
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

    fn delete_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
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
                    let files = client.list_files(&config.tokens.access_token, 100).await?;
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

    fn move_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
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
                    let files = client.list_files(&config.tokens.access_token, 100).await?;
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

    fn download_remote_file(&mut self, file: FileNode, cx: &mut Context<Self>) {
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

    fn refresh_trash(&mut self, cx: &mut Context<Self>) {
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

    fn restore_trash_entry(&mut self, entry: TrashEntry, cx: &mut Context<Self>) {
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

    fn refresh_conflicts(&mut self, cx: &mut Context<Self>) {
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

    fn refresh_devices(&mut self, cx: &mut Context<Self>) {
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

    fn resolve_conflict(
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

    fn run_sync_command(&mut self, action: &'static str, cx: &mut Context<Self>) {
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

    fn run_daemon_command(&mut self, action: &str, cx: &mut Context<Self>) {
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

    fn init_workspace(&mut self, cx: &mut Context<Self>) {
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

    fn save_server(&mut self, cx: &mut Context<Self>) {
        self.settings.server_url = self.current_server(cx);
        match save_settings(&self.settings) {
            Ok(()) => self.set_message("server saved", cx),
            Err(error) => self.set_message(format!("save settings failed: {error}"), cx),
        }
    }

    fn render_auth_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let signed_in = self.cli_config.is_some();
        v_flex()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(Icon::new(IconName::Globe).text_color(colors.accent))
                    .child(
                        Label::new("SyncHub Desktop")
                            .text_color(colors.text)
                            .text_size(rems(1.1)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-all")
                            .icon(IconName::Redo2)
                            .ghost()
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.refresh_all(window, cx)),
                            ),
                    )
                    .when(signed_in, |this| {
                        this.child(
                            Button::new("logout")
                                .icon(IconName::CircleX)
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.logout(cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(280.)).child(Input::new(&self.server_input)))
                    .child(
                        Button::new("save-server")
                            .icon(IconName::Check)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.save_server(cx))),
                    )
                    .child(
                        Button::new("check-api")
                            .icon(IconName::Info)
                            .label("Ready")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_api(cx))),
                    )
                    .child(
                        self.render_status_badge(self.api_status.as_deref().unwrap_or("unchecked")),
                    ),
            )
            .when(!signed_in, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(220.)).child(Input::new(&self.email_input)))
                        .child(div().w(px(180.)).child(Input::new(&self.password_input)))
                        .child(
                            Button::new("login")
                                .icon(IconName::User)
                                .label("Login")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.authenticate(AuthMode::Login, cx)
                                })),
                        )
                        .child(
                            Button::new("register")
                                .icon(IconName::Plus)
                                .label("Register")
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.authenticate(AuthMode::Register, cx)
                                })),
                        ),
                )
            })
            .when(signed_in, |this| {
                let label = self
                    .cli_config
                    .as_ref()
                    .map(|config| format!("{} via {}", config.user.email, config.server_url))
                    .unwrap_or_default();
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::User).small().text_color(colors.success))
                        .child(Label::new(label).text_color(colors.muted)),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        v_flex()
            .w(px(280.))
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.panel_alt)
            .child(
                h_flex()
                    .p_3()
                    .items_center()
                    .justify_between()
                    .child(Label::new("Workspaces").text_color(colors.text))
                    .child(
                        Button::new("reload-workspaces")
                            .icon(IconName::Redo2)
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_local_state(window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .px_3()
                    .pb_3()
                    .gap_2()
                    .child(Input::new(&self.workspace_input))
                    .child(Input::new(&self.remote_root_input))
                    .child(
                        Button::new("init-workspace")
                            .icon(IconName::Plus)
                            .label("Init Selected")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.init_workspace(cx))),
                    ),
            )
            .child(
                v_flex().flex_1().overflow_y_scrollbar().children(
                    self.workspaces
                        .iter()
                        .enumerate()
                        .map(|(index, workspace)| {
                            let selected = index == self.selected_workspace;
                            let metrics = workspace_metrics(workspace);
                            let title = workspace.display_name();
                            let remote = workspace.remote_path();
                            let root = workspace.root_path().display().to_string();
                            v_flex()
                                .id(("workspace", index))
                                .mx_2()
                                .mb_2()
                                .p_3()
                                .gap_1()
                                .rounded_md()
                                .border_1()
                                .border_color(if selected {
                                    colors.accent
                                } else {
                                    colors.border
                                })
                                .bg(if selected {
                                    alpha(colors.accent, 0.08)
                                } else {
                                    colors.panel
                                })
                                .cursor_pointer()
                                .hover(|style| style.border_color(colors.accent))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected_workspace = index;
                                    this.active_view = MainView::Overview;
                                    cx.notify();
                                }))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            Icon::new(IconName::Folder)
                                                .small()
                                                .text_color(colors.accent),
                                        )
                                        .child(Label::new(title).text_color(colors.text)),
                                )
                                .child(
                                    Label::new(remote)
                                        .text_color(colors.muted)
                                        .text_size(rems(0.78)),
                                )
                                .child(
                                    Label::new(root)
                                        .text_color(colors.muted)
                                        .text_size(rems(0.72)),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .mt_1()
                                        .child(
                                            self.render_tiny_metric(
                                                "files",
                                                metrics.manifest_files,
                                            ),
                                        )
                                        .child(
                                            self.render_tiny_metric("trash", metrics.trash_entries),
                                        ),
                                )
                        }),
                ),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        h_flex()
            .h(px(42.))
            .gap_1()
            .px_3()
            .items_center()
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.panel)
            .child(self.render_nav_button(
                "overview",
                MainView::Overview,
                IconName::LayoutDashboard,
                "Overview",
                cx,
            ))
            .child(self.render_nav_button("files", MainView::Files, IconName::File, "Files", cx))
            .child(self.render_nav_button("trash", MainView::Trash, IconName::Inbox, "Trash", cx))
            .child(self.render_nav_button("sync", MainView::Sync, IconName::Redo2, "Sync", cx))
            .child(self.render_nav_button(
                "devices",
                MainView::Devices,
                IconName::HardDrive,
                "Devices",
                cx,
            ))
            .child(self.render_nav_button(
                "conflicts",
                MainView::Conflicts,
                IconName::TriangleAlert,
                "Conflicts",
                cx,
            ))
            .child(self.render_nav_button(
                "daemon",
                MainView::Daemon,
                IconName::SquareTerminal,
                "Daemon",
                cx,
            ))
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.active_view {
            MainView::Overview => self.render_overview(cx).into_any_element(),
            MainView::Sync => self.render_sync(cx).into_any_element(),
            MainView::Files => self.render_files(cx).into_any_element(),
            MainView::Trash => self.render_trash(cx).into_any_element(),
            MainView::Devices => self.render_devices(cx).into_any_element(),
            MainView::Conflicts => self.render_conflicts(cx).into_any_element(),
            MainView::Daemon => self.render_daemon(cx).into_any_element(),
        }
    }

    fn render_overview(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let metrics = self
            .current_workspace()
            .map(workspace_metrics)
            .unwrap_or_default();
        v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .bg(colors.bg)
            .child(
                h_flex()
                    .gap_3()
                    .child(self.render_metric_tile(
                        "Manifest files",
                        metrics.manifest_files.to_string(),
                        IconName::File,
                        colors.accent,
                    ))
                    .child(self.render_metric_tile(
                        "Remote tracked",
                        metrics.remote_tracked.to_string(),
                        IconName::Globe,
                        colors.success,
                    ))
                    .child(self.render_metric_tile(
                        "Local only",
                        metrics.local_only.to_string(),
                        IconName::HardDrive,
                        colors.warning,
                    ))
                    .child(self.render_metric_tile(
                        "Trash",
                        metrics.trash_entries.to_string(),
                        IconName::Inbox,
                        colors.danger,
                    )),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .child(
                        Label::new("Workspace")
                            .text_color(colors.text)
                            .text_size(rems(1.0)),
                    )
                    .child(self.render_workspace_detail()),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .child(
                        Label::new("Last action")
                            .text_color(colors.text)
                            .text_size(rems(1.0)),
                    )
                    .child(
                        Label::new(if self.message.is_empty() {
                            "Ready"
                        } else {
                            &self.message
                        })
                        .text_color(colors.muted),
                    )
                    .when_some(self.command_result.as_ref(), |this, result| {
                        this.child(
                            Label::new(result.output.as_str())
                                .text_color(colors.muted)
                                .text_size(rems(0.78)),
                        )
                    }),
            )
    }

    fn render_sync(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let controls = h_flex()
            .gap_2()
            .child(self.render_sync_button("sync-once", "once", IconName::Redo2, false, cx))
            .child(self.render_sync_button("sync-dry-run", "dry-run", IconName::Search, true, cx))
            .child(self.render_sync_button("sync-push", "push", IconName::ArrowUp, true, cx))
            .child(self.render_sync_button("sync-pull", "pull", IconName::ArrowDown, true, cx))
            .child(self.render_sync_button("sync-status", "status", IconName::Info, true, cx))
            .child(self.render_sync_button("sync-doctor", "doctor", IconName::Check, true, cx))
            .into_any_element();
        let workspace = v_flex()
            .gap_2()
            .p_4()
            .bg(colors.panel)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .child(Label::new("Workspace").text_color(colors.text))
            .child(self.render_workspace_detail())
            .into_any_element();
        let command_output = v_flex()
            .gap_2()
            .p_4()
            .flex_1()
            .overflow_y_scrollbar()
            .bg(colors.panel)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .child(Label::new("Last sync command").text_color(colors.text))
            .child(
                Label::new(if self.message.is_empty() {
                    "Ready"
                } else {
                    &self.message
                })
                .text_color(colors.muted),
            )
            .when_some(self.command_result.as_ref(), |this, result| {
                this.child(
                    Label::new(result.output.as_str())
                        .text_color(colors.muted)
                        .text_size(rems(0.78)),
                )
            })
            .into_any_element();

        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .bg(colors.bg)
            .child(controls)
            .child(workspace)
            .child(command_output)
            .into_any_element()
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let file_rows = self
            .files
            .iter()
            .map(|file| self.render_file_row(file, cx).into_any_element())
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .justify_between()
                    .items_center()
                    .child(Label::new("Remote Files").text_color(colors.text))
                    .child(div().flex_1())
                    .child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&self.remote_directory_input)),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .child(Input::new(&self.remote_target_input)),
                    )
                    .child(
                        Button::new("create-remote-directory")
                            .icon(IconName::Plus)
                            .label("New Folder")
                            .small()
                            .disabled(self.loading)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.create_remote_directory(cx)),
                            ),
                    )
                    .child(
                        Button::new("load-files")
                            .icon(IconName::Redo2)
                            .label("Load")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_files(cx))),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .mx_3()
                    .mb_3()
                    .overflow_y_scrollbar()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .children(file_rows),
            )
    }

    fn render_trash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let trash_rows = self
            .trash_entries
            .iter()
            .map(|entry| self.render_trash_row(entry, cx).into_any_element())
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                h_flex()
                    .p_3()
                    .justify_between()
                    .items_center()
                    .child(Label::new("Local Trash").text_color(colors.text))
                    .child(
                        Button::new("load-trash")
                            .icon(IconName::Redo2)
                            .label("Load")
                            .small()
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_trash(cx))),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .mx_3()
                    .mb_3()
                    .overflow_y_scrollbar()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .children(trash_rows),
            )
    }

    fn render_devices(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let device_rows = self
            .devices
            .iter()
            .map(|device| self.render_device_row(device).into_any_element())
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                h_flex()
                    .p_3()
                    .justify_between()
                    .items_center()
                    .child(Label::new("Devices").text_color(colors.text))
                    .child(
                        Button::new("load-devices")
                            .icon(IconName::Redo2)
                            .label("Load")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_devices(cx))),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .mx_3()
                    .mb_3()
                    .overflow_y_scrollbar()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .children(device_rows),
            )
    }

    fn render_conflicts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let conflict_rows = self
            .conflicts
            .iter()
            .map(|conflict| self.render_conflict_row(conflict, cx).into_any_element())
            .collect::<Vec<_>>();

        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                h_flex()
                    .p_3()
                    .justify_between()
                    .items_center()
                    .child(Label::new("Pending Conflicts").text_color(colors.text))
                    .child(
                        Button::new("load-conflicts")
                            .icon(IconName::Redo2)
                            .label("Load")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_conflicts(cx))),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .mx_3()
                    .mb_3()
                    .overflow_y_scrollbar()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .children(conflict_rows),
            )
    }

    fn render_daemon(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .bg(colors.bg)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("daemon-start")
                            .icon(IconName::Play)
                            .label("Start")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.run_daemon_command("start", cx)),
                            ),
                    )
                    .child(
                        Button::new("daemon-status")
                            .icon(IconName::Info)
                            .label("Status")
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.run_daemon_command("status", cx)),
                            ),
                    )
                    .child(
                        Button::new("daemon-pause")
                            .icon(IconName::Pause)
                            .label("Pause")
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.run_daemon_command("pause", cx)),
                            ),
                    )
                    .child(
                        Button::new("daemon-resume")
                            .icon(IconName::Redo2)
                            .label("Resume")
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.run_daemon_command("resume", cx)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .child(Label::new("Daemon state").text_color(colors.text))
                    .child(self.render_daemon_state()),
            )
            .when_some(self.command_result.as_ref(), |this, result| {
                this.child(
                    v_flex()
                        .gap_2()
                        .p_4()
                        .bg(colors.panel)
                        .border_1()
                        .border_color(if result.ok {
                            colors.success
                        } else {
                            colors.danger
                        })
                        .rounded_md()
                        .child(Label::new(result.summary.as_str()).text_color(colors.text))
                        .child(
                            Label::new(result.output.as_str())
                                .text_color(colors.muted)
                                .text_size(rems(0.78)),
                        ),
                )
            })
    }

    fn render_workspace_detail(&self) -> impl IntoElement {
        let colors = self.colors;
        if let Some(workspace) = self.current_workspace() {
            let device = workspace.device_id();
            let manifest_time = workspace
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.generated_at.clone())
                .unwrap_or_else(|| "-".to_string());
            v_flex()
                .gap_2()
                .child(self.render_detail_row("Root", workspace.root_path().display().to_string()))
                .child(self.render_detail_row("Remote", workspace.remote_path()))
                .child(self.render_detail_row(
                    "Device",
                    if device.is_empty() {
                        "-".to_string()
                    } else {
                        device
                    },
                ))
                .child(self.render_detail_row("Manifest", manifest_time))
                .when_some(workspace.config_error.as_ref(), |this, error| {
                    this.child(Label::new(error.as_str()).text_color(colors.danger))
                })
                .into_any_element()
        } else {
            v_flex()
                .gap_2()
                .child(Label::new("No initialized workspace found").text_color(colors.muted))
                .into_any_element()
        }
    }

    fn render_daemon_state(&self) -> impl IntoElement {
        let colors = self.colors;
        if let Some(workspace) = self.current_workspace() {
            let state = workspace.daemon_state.as_ref();
            let control = workspace.daemon_control.as_ref();
            v_flex()
                .gap_2()
                .child(
                    self.render_detail_row(
                        "Status",
                        state
                            .map(|state| state.status.clone())
                            .filter(|status| !status.is_empty())
                            .unwrap_or_else(|| "not run".to_string()),
                    ),
                )
                .child(
                    self.render_detail_row(
                        "Cycles",
                        state
                            .map(|state| state.cycles_run.to_string())
                            .unwrap_or_else(|| "0".to_string()),
                    ),
                )
                .child(
                    self.render_detail_row(
                        "Failures",
                        state
                            .map(|state| state.consecutive_failures.to_string())
                            .unwrap_or_else(|| "0".to_string()),
                    ),
                )
                .child(
                    self.render_detail_row(
                        "Paused",
                        control
                            .map(|control| if control.paused { "yes" } else { "no" }.to_string())
                            .unwrap_or_else(|| "no".to_string()),
                    ),
                )
                .when_some(
                    state.and_then(|state| {
                        (!state.last_error.is_empty()).then_some(state.last_error.as_str())
                    }),
                    |this, error| this.child(Label::new(error).text_color(colors.danger)),
                )
                .into_any_element()
        } else {
            Label::new("Select a workspace first")
                .text_color(colors.muted)
                .into_any_element()
        }
    }

    fn render_detail_row(&self, key: &str, value: String) -> impl IntoElement {
        let colors = self.colors;
        h_flex()
            .gap_3()
            .items_start()
            .child(
                div()
                    .w(px(88.))
                    .child(Label::new(key).text_color(colors.muted)),
            )
            .child(Label::new(value).text_color(colors.text))
    }

    fn render_trash_row(&self, entry: &TrashEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let entry_for_restore = entry.clone();
        h_flex()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(if entry.is_dir {
                    IconName::FolderOpen
                } else {
                    IconName::File
                })
                .small()
                .text_color(colors.warning),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Label::new(entry.path.as_str()).text_color(colors.text))
                    .child(
                        Label::new(entry.batch.as_str())
                            .text_color(colors.muted)
                            .text_size(rems(0.72)),
                    ),
            )
            .child(
                Label::new(if entry.is_dir {
                    "-".to_string()
                } else {
                    format_bytes(entry.size)
                })
                .text_color(colors.muted),
            )
            .child(self.render_status_badge(if entry.is_dir { "directory" } else { "file" }))
            .child(
                Button::new(format!("restore-trash-{}-{}", entry.batch, entry.path))
                    .icon(IconName::Undo2)
                    .label("Restore")
                    .ghost()
                    .small()
                    .disabled(self.loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.restore_trash_entry(entry_for_restore.clone(), cx)
                    })),
            )
    }

    fn render_file_row(&self, file: &FileNode, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let file_for_download = file.clone();
        let file_for_move = file.clone();
        let file_for_delete = file.clone();
        h_flex()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(if file.node_type == "directory" {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small()
                .text_color(colors.accent),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Label::new(file.path.as_str()).text_color(colors.text))
                    .child(
                        Label::new(file.id.as_str())
                            .text_color(colors.muted)
                            .text_size(rems(0.72)),
                    ),
            )
            .child(Label::new(format_bytes(file.size)).text_color(colors.muted))
            .child(self.render_status_badge(format!("v{}", file.version).as_str()))
            .child(
                Button::new(format!("download-file-{}", file.id))
                    .icon(IconName::ArrowDown)
                    .ghost()
                    .small()
                    .disabled(self.loading || file.node_type != "file")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.download_remote_file(file_for_download.clone(), cx)
                    })),
            )
            .child(
                Button::new(format!("move-file-{}", file.id))
                    .icon(IconName::ArrowRight)
                    .ghost()
                    .small()
                    .disabled(self.loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.move_remote_file(file_for_move.clone(), cx)
                    })),
            )
            .child(
                Button::new(format!("delete-file-{}", file.id))
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .disabled(self.loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_remote_file(file_for_delete.clone(), cx)
                    })),
            )
    }

    fn render_device_row(&self, device: &Device) -> impl IntoElement {
        let colors = self.colors;
        let current = self
            .current_workspace()
            .map(|workspace| is_current_device(device, workspace))
            .unwrap_or(false);
        h_flex()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::HardDrive)
                    .small()
                    .text_color(if current {
                        colors.success
                    } else {
                        colors.accent
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Label::new(device_name(device)).text_color(colors.text))
                    .child(
                        Label::new(device.id.as_str())
                            .text_color(colors.muted)
                            .text_size(rems(0.72)),
                    ),
            )
            .child(Label::new(device.platform.as_str()).text_color(colors.muted))
            .child(
                Label::new(format!("cursor {}", device.last_applied_change_id))
                    .text_color(colors.muted),
            )
            .child(
                Label::new(format_optional(device.last_seen_at.as_deref()))
                    .text_color(colors.muted),
            )
            .when(current, |this| {
                this.child(self.render_status_badge("current"))
            })
    }

    fn render_conflict_row(
        &self,
        conflict: &SyncConflict,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors;
        let keep_local_id = conflict.id.clone();
        let keep_remote_id = conflict.id.clone();
        let keep_both_id = conflict.id.clone();
        h_flex()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(colors.warning),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Label::new(conflict.path.as_str()).text_color(colors.text))
                    .child(
                        Label::new(conflict.id.as_str())
                            .text_color(colors.muted)
                            .text_size(rems(0.72)),
                    ),
            )
            .child(
                Label::new(format!("local {}", optional_i64(conflict.local_version)))
                    .text_color(colors.muted),
            )
            .child(
                Label::new(format!("remote {}", optional_i64(conflict.remote_version)))
                    .text_color(colors.muted),
            )
            .child(self.render_status_badge(conflict.resolution.as_str()))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new(format!("keep-local-{}", conflict.id))
                            .icon(IconName::HardDrive)
                            .label("Local")
                            .ghost()
                            .small()
                            .disabled(self.loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.resolve_conflict(keep_local_id.clone(), "keep_local", cx)
                            })),
                    )
                    .child(
                        Button::new(format!("keep-remote-{}", conflict.id))
                            .icon(IconName::Globe)
                            .label("Remote")
                            .ghost()
                            .small()
                            .disabled(self.loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.resolve_conflict(keep_remote_id.clone(), "keep_remote", cx)
                            })),
                    )
                    .child(
                        Button::new(format!("keep-both-{}", conflict.id))
                            .icon(IconName::Copy)
                            .label("Both")
                            .ghost()
                            .small()
                            .disabled(self.loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.resolve_conflict(keep_both_id.clone(), "keep_both", cx)
                            })),
                    ),
            )
    }

    fn render_metric_tile(
        &self,
        title: &str,
        value: String,
        icon: IconName,
        color: Hsla,
    ) -> impl IntoElement {
        let colors = self.colors;
        v_flex()
            .flex_1()
            .gap_2()
            .p_4()
            .bg(colors.panel)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(Label::new(title).text_color(colors.muted))
                    .child(Icon::new(icon).text_color(color)),
            )
            .child(
                Label::new(value)
                    .text_color(colors.text)
                    .text_size(rems(1.5)),
            )
    }

    fn render_tiny_metric(&self, name: &str, value: usize) -> impl IntoElement {
        let colors = self.colors;
        h_flex()
            .gap_1()
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(alpha(colors.muted, 0.08))
            .child(
                Label::new(format!("{name} {value}"))
                    .text_color(colors.muted)
                    .text_size(rems(0.72)),
            )
    }

    fn render_sync_button(
        &self,
        id: &'static str,
        action: &'static str,
        icon: IconName,
        ghost: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(id)
            .icon(icon)
            .label(sync_action_label(action))
            .when(ghost, |button| button.ghost())
            .disabled(self.loading)
            .on_click(cx.listener(move |this, _, _, cx| this.run_sync_command(action, cx)))
            .into_any_element()
    }

    fn render_status_badge(&self, text: &str) -> impl IntoElement {
        let colors = self.colors;
        let lower = text.to_ascii_lowercase();
        let color = if lower.contains("ready") || lower == "ok" || lower.starts_with('v') {
            colors.success
        } else if lower.contains("unchecked") || lower.contains("pending") || lower.contains("not")
        {
            colors.warning
        } else if lower.contains("unreachable") || lower.contains("error") || lower.contains("fail")
        {
            colors.danger
        } else {
            colors.accent
        };
        h_flex()
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(alpha(color, 0.12))
            .child(Label::new(text).text_color(color).text_size(rems(0.75)))
    }

    fn render_nav_button(
        &self,
        id: &'static str,
        view: MainView,
        icon: IconName,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors;
        Button::new(id)
            .icon(icon)
            .label(label)
            .ghost()
            .small()
            .when(self.active_view == view, |button| {
                button
                    .bg(alpha(colors.accent, 0.10))
                    .text_color(colors.accent)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.active_view = view;
                cx.notify();
            }))
    }
}

impl Render for SyncHubDesktop {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                TitleBar::new().child(div().flex_1()).child(
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("title-refresh")
                                .icon(if self.loading {
                                    IconName::Loader
                                } else {
                                    IconName::Redo2
                                })
                                .ghost()
                                .small()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh_all(window, cx)),
                                ),
                        ),
                ),
            )
            .child(self.render_auth_panel(cx))
            .child(
                h_flex()
                    .flex_1()
                    .size_full()
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .size_full()
                            .child(self.render_tabs(cx))
                            .child(self.render_content(cx)),
                    ),
            )
    }
}

fn run_synchub_cli_file_download(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
    config_path: &PathBuf,
    file: &FileNode,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let workspace_config = workspace_config.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = file_download_command_args(&root, &workspace_config, &config, &file.id) else {
        return CommandResult {
            ok: false,
            summary: "remote file id is required".to_string(),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    if result.ok {
        result.summary = format!("downloaded remote file {}", file.path);
    } else {
        result.summary = format!("download failed: {}", result.summary);
    }
    result
}

fn run_synchub_cli_trash_list(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
) -> (CommandResult, Vec<TrashEntry>) {
    let root = workspace_root.display().to_string();
    let config = workspace_config.display().to_string();
    let Some(args) = trash_list_command_args(&root, &config, 200) else {
        return (
            CommandResult {
                ok: false,
                summary: "workspace path is required".to_string(),
                output: String::new(),
            },
            Vec::new(),
        );
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    let entries = if result.ok {
        match serde_json::from_str::<SyncTrashSnapshot>(&result.output) {
            Ok(snapshot) => snapshot.items,
            Err(error) => {
                result.ok = false;
                result.summary = format!("decode trash failed: {error}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if result.ok {
        result.summary = format!("loaded {} trash item(s)", entries.len());
    } else {
        result.summary = format!("load trash failed: {}", result.summary);
    }
    (result, entries)
}

fn run_synchub_cli_trash_restore(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
    entry: &TrashEntry,
) -> (CommandResult, Option<Vec<TrashEntry>>) {
    let root = workspace_root.display().to_string();
    let config = workspace_config.display().to_string();
    let Some(args) = trash_restore_command_args(&root, &config, &entry.batch, &entry.path) else {
        return (
            CommandResult {
                ok: false,
                summary: "trash batch and entry are required".to_string(),
                output: String::new(),
            },
            None,
        );
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    if !result.ok {
        result.summary = format!("restore trash failed: {}", result.summary);
        return (result, None);
    }

    let restore_output = result.output.clone();
    let (list_result, entries) = run_synchub_cli_trash_list(workspace_root, workspace_config);
    result.output = if list_result.output.trim().is_empty() {
        restore_output
    } else {
        format!("{restore_output}\n{}", list_result.output)
    };
    result.ok = list_result.ok;
    result.summary = if list_result.ok {
        format!("restored trash item {}", entry.path)
    } else {
        format!(
            "restored {}, but refresh failed: {}",
            entry.path, list_result.summary
        )
    };
    (result, Some(entries))
}

fn run_synchub_cli_daemon(
    action: &str,
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let mut args = vec!["sync", "daemon"];
    match action {
        "status" => args.push("--status"),
        "pause" => args.push("--pause"),
        "resume" => args.push("--resume"),
        "start" => {}
        _ => {}
    }
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    args.extend(["--path", &root, "--config", &config]);
    run_command("synchub-cli", &args)
}

fn run_synchub_cli_workspace_init(
    roots: &[String],
    remote_root: &str,
    config_path: &PathBuf,
) -> CommandResult {
    let config = config_path.display().to_string();
    let Some(args) = workspace_init_command_args(roots, remote_root, &config) else {
        return CommandResult {
            ok: false,
            summary: "workspace path is required".to_string(),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("initialized {} workspace(s)", roots.len())
    } else {
        format!("workspace init failed: {}", result.summary)
    };
    result
}

fn run_synchub_cli_sync(
    action: &str,
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = sync_command_args(action, &root, &config) else {
        return CommandResult {
            ok: false,
            summary: format!("unknown sync action: {action}"),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("{} completed", sync_action_label(action))
    } else {
        format!("{} failed: {}", sync_action_label(action), result.summary)
    };
    result
}

fn run_command(program: &str, args: &[&str]) -> CommandResult {
    let output = Command::new(program).args(args).output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.trim().is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}{stderr}")
            };
            CommandResult {
                ok: output.status.success(),
                summary: if output.status.success() {
                    "command completed".to_string()
                } else {
                    format!("command exited with {}", output.status)
                },
                output: combined,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            summary: format!("failed to start {program}: {error}"),
            output: String::new(),
        },
    }
}

fn device_name(device: &Device) -> &str {
    if device.name.trim().is_empty() {
        "unnamed device"
    } else {
        device.name.as_str()
    }
}

fn format_optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "-".to_string())
}

fn optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn rfc3339_from_system_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (year, month, day, hour, minute, second) = civil_from_unix(seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_unix(seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + (month <= 2) as i64;
    (year, month, day)
}

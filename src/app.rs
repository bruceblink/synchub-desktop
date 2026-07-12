mod controller;
mod formatting;
pub(crate) mod time;
mod view;

use crate::config::{
    DesktopSettings, default_cli_config_path, default_workspace_registry_path, load_cli_config,
    load_settings_with_legacy_cli, load_workspace_snapshots, save_settings, update_cli_server_url,
    update_workspace_server_urls,
};
use crate::models::{
    ApiStatus, CliConfig, Device, FileNode, FileVersion, SyncConflict, TrashEntry, VersionInfo,
    WorkspaceSnapshot,
};
use crate::theme::ThemeColors;
use gpui::*;
use gpui_component::input::InputState;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MainView {
    Overview,
    Server,
    Sync,
    Files,
    Versions,
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
    server_version: Option<VersionInfo>,
    server_health: Option<ApiStatus>,
    server_ready: Option<ApiStatus>,
    server_result: Option<CommandResult>,
    files: Vec<FileNode>,
    files_next_cursor: Option<String>,
    selected_file: Option<FileNode>,
    file_versions: Vec<FileVersion>,
    trash_entries: Vec<TrashEntry>,
    cloud_trash: Vec<FileNode>,
    devices: Vec<Device>,
    conflicts: Vec<SyncConflict>,
    active_view: MainView,
    auth_mode: AuthMode,
    initializing: bool,
    loading: bool,
    message: String,
    command_result: Option<CommandResult>,
    daemon_tasks: HashMap<String, JoinHandle<()>>,
    colors: ThemeColors,
}

impl SyncHubDesktop {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cli_config_path = default_cli_config_path();
        let settings = DesktopSettings::default();
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
            server_version: None,
            server_health: None,
            server_ready: None,
            server_result: None,
            files: Vec::new(),
            files_next_cursor: None,
            selected_file: None,
            file_versions: Vec::new(),
            trash_entries: Vec::new(),
            cloud_trash: Vec::new(),
            devices: Vec::new(),
            conflicts: Vec::new(),
            active_view: MainView::Overview,
            auth_mode: AuthMode::Login,
            initializing: true,
            loading: false,
            message: String::new(),
            command_result: None,
            daemon_tasks: HashMap::new(),
            colors: ThemeColors::default(),
        };
        app.initialize(window, cx);
        app
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cli_config_path = self.cli_config_path.clone();
        let registry_path = self.registry_path.clone();
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let loaded = tokio::task::spawn_blocking(move || {
                let mut message = String::new();
                let mut settings = load_settings_with_legacy_cli(&cli_config_path);
                let _ = save_settings(&settings);
                settings.server_url = crate::client::normalize_base_url(&settings.server_url);

                let mut cli_config = match load_cli_config(&cli_config_path) {
                    Ok(config) => config,
                    Err(error) => {
                        message = format!("read login config failed: {error}");
                        None
                    }
                };
                if cli_config.is_some() {
                    match update_cli_server_url(&cli_config_path, &settings.server_url) {
                        Ok(config) => cli_config = config,
                        Err(error) => message = format!("update login server failed: {error}"),
                    }
                }
                if let Err(error) =
                    update_workspace_server_urls(&registry_path, &settings.server_url)
                {
                    message = format!("update workspace server failed: {error}");
                }
                let workspaces = match load_workspace_snapshots(&registry_path) {
                    Ok(workspaces) => workspaces,
                    Err(error) => {
                        message = format!("read workspace registry failed: {error}");
                        Vec::new()
                    }
                };
                (settings, cli_config, workspaces, message)
            })
            .await;

            if let Some(this) = this.upgrade() {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.initializing = false;
                    match loaded {
                        Ok((settings, cli_config, workspaces, message)) => {
                            this.settings = settings;
                            this.cli_config = cli_config;
                            this.workspaces = workspaces;
                            this.message = message;
                            this.server_input.update(cx, |input, cx| {
                                input.set_value(this.settings.server_url.clone(), window, cx);
                            });
                            if let Some(workspace) = this.current_workspace() {
                                let root = workspace.root_path().display().to_string();
                                this.workspace_input.update(cx, |input, cx| {
                                    input.set_value(root, window, cx);
                                });
                            }
                            this.start_registered_workspace_daemons();
                            this.check_api_status(cx);
                        }
                        Err(error) => {
                            this.message = format!("initialize desktop failed: {error}");
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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
}

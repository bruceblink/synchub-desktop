mod commands;
mod controller;
mod formatting;
mod time;
mod view;

use crate::config::{
    DesktopSettings, default_cli_config_path, default_workspace_registry_path, load_settings,
};
use crate::models::{CliConfig, Device, FileNode, SyncConflict, TrashEntry, WorkspaceSnapshot};
use crate::theme::ThemeColors;
use gpui::*;
use gpui_component::input::InputState;
use std::path::PathBuf;

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
}

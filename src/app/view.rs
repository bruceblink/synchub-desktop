use super::formatting::{
    device_name, format_optional, optional_i64, short_hash, sorted_status_checks,
};
use super::{AuthMode, MainView, SyncHubDesktop};
use crate::models::{
    Device, FileNode, FileVersion, SyncConflict, TrashEntry, file_version_label, format_bytes,
    is_current_device, is_file_version_pinned, workspace_metrics,
};
use crate::sync_commands::sync_action_label;
use crate::theme::alpha;
use gpui::prelude::*;
use gpui::*;
use gpui_component::{
    Icon, IconName, TitleBar, button::*, input::Input, label::Label, scroll::ScrollableElement, *,
};
impl SyncHubDesktop {
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
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("init-workspace")
                                    .icon(IconName::Plus)
                                    .label("Init")
                                    .small()
                                    .ghost()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.init_workspace(cx)),
                                    ),
                            )
                            .child(
                                Button::new("remove-workspace")
                                    .icon(IconName::Close)
                                    .label("Remove")
                                    .small()
                                    .ghost()
                                    .disabled(self.loading || self.workspaces.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.remove_selected_workspace(cx)
                                    })),
                            )
                            .child(
                                Button::new("prune-workspaces")
                                    .icon(IconName::Search)
                                    .label("Prune")
                                    .small()
                                    .ghost()
                                    .disabled(self.loading)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.prune_workspaces(cx)),
                                    ),
                            ),
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
            .child(self.render_nav_button(
                "server",
                MainView::Server,
                IconName::Globe,
                "Server",
                cx,
            ))
            .child(self.render_nav_button("files", MainView::Files, IconName::File, "Files", cx))
            .child(self.render_nav_button(
                "versions",
                MainView::Versions,
                IconName::Calendar,
                "Versions",
                cx,
            ))
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
            MainView::Server => self.render_server(cx).into_any_element(),
            MainView::Sync => self.render_sync(cx).into_any_element(),
            MainView::Files => self.render_files(cx).into_any_element(),
            MainView::Versions => self.render_versions(cx).into_any_element(),
            MainView::Trash => self.render_trash(cx).into_any_element(),
            MainView::Devices => self.render_devices(cx).into_any_element(),
            MainView::Conflicts => self.render_conflicts(cx).into_any_element(),
            MainView::Daemon => self.render_daemon(cx).into_any_element(),
        }
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                        "Pending",
                        metrics.pending_local_changes.to_string(),
                        IconName::Search,
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
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Label::new("Workspace")
                                    .text_color(colors.text)
                                    .text_size(rems(1.0)),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("scan-manifest")
                                    .icon(IconName::Search)
                                    .label("Scan Manifest")
                                    .small()
                                    .ghost()
                                    .disabled(self.loading || self.current_workspace().is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.scan_selected_manifest(cx)
                                    })),
                            ),
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

    fn render_server(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let version = self
            .server_version
            .as_ref()
            .map(|version| format!("{} {}", version.name, version.version))
            .unwrap_or_else(|| "-".to_string());
        let health = self
            .server_health
            .as_ref()
            .map(|status| status.status.clone())
            .filter(|status| !status.is_empty())
            .unwrap_or_else(|| "unchecked".to_string());
        let ready = self
            .server_ready
            .as_ref()
            .map(|status| status.status.clone())
            .filter(|status| !status.is_empty())
            .unwrap_or_else(|| "unchecked".to_string());
        let readiness_checks = self
            .server_ready
            .as_ref()
            .map(|status| sorted_status_checks(&status.checks))
            .unwrap_or_default();

        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .bg(colors.bg)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Label::new("Server").text_color(colors.text))
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-server-status")
                            .icon(IconName::Redo2)
                            .label("Refresh")
                            .small()
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_server_status(cx))),
                    )
                    .child(
                        Button::new("server-metrics")
                            .icon(IconName::Info)
                            .label("Metrics")
                            .small()
                            .ghost()
                            .disabled(self.loading)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.refresh_server_metrics(cx)),
                            ),
                    )
                    .child(
                        Button::new("server-openapi")
                            .icon(IconName::File)
                            .label("OpenAPI")
                            .small()
                            .ghost()
                            .disabled(self.loading)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.refresh_server_openapi(cx)),
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
                    .child(self.render_detail_row("URL", self.current_server(cx)))
                    .child(self.render_detail_row("Version", version))
                    .child(self.render_detail_row("Health", health.clone()))
                    .child(self.render_detail_row("Ready", ready.clone()))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.render_status_badge(health.as_str()))
                            .child(self.render_status_badge(ready.as_str())),
                    )
                    .when(!readiness_checks.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .gap_2()
                                .pt_2()
                                .mt_1()
                                .border_t_1()
                                .border_color(colors.border)
                                .child(
                                    Label::new("Readiness checks")
                                        .text_color(colors.muted)
                                        .text_size(rems(0.82)),
                                )
                                .children(readiness_checks.iter().map(|(name, status)| {
                                    self.render_readiness_check_row(name, status)
                                })),
                        )
                    }),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(colors.panel)
                    .border_1()
                    .border_color(colors.border)
                    .rounded_md()
                    .child(Label::new("Last check").text_color(colors.text))
                    .child(
                        Label::new(if self.message.is_empty() {
                            "Ready"
                        } else {
                            &self.message
                        })
                        .text_color(colors.muted),
                    ),
            )
            .when_some(self.server_result.as_ref(), |this, result| {
                this.child(
                    v_flex()
                        .gap_2()
                        .p_4()
                        .flex_1()
                        .overflow_y_scrollbar()
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
                    )
                    .child(
                        Button::new("load-more-files")
                            .icon(IconName::ChevronDown)
                            .label("More")
                            .small()
                            .ghost()
                            .disabled(self.loading || self.files_next_cursor.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.load_more_files(cx))),
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

    fn render_versions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors;
        let version_rows = self
            .file_versions
            .iter()
            .map(|version| self.render_file_version_row(version, cx).into_any_element())
            .collect::<Vec<_>>();
        let target = self
            .selected_file
            .as_ref()
            .map(|file| file.path.as_str())
            .unwrap_or("No remote file selected");

        v_flex()
            .size_full()
            .bg(colors.bg)
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .justify_between()
                    .items_center()
                    .child(Label::new("File Versions").text_color(colors.text))
                    .child(Label::new(target).text_color(colors.muted))
                    .child(div().flex_1())
                    .child(
                        Button::new("load-file-versions")
                            .icon(IconName::Redo2)
                            .label("Load")
                            .small()
                            .disabled(self.loading || self.selected_file.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_selected_file_versions(cx)
                            })),
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
                    .children(version_rows),
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
                h_flex().gap_2().child(
                    Button::new("daemon-reset-state")
                        .icon(IconName::Undo2)
                        .label("Reset State")
                        .ghost()
                        .disabled(self.loading || self.current_workspace().is_none())
                        .on_click(
                            cx.listener(|this, _, _, cx| {
                                this.run_daemon_command("reset-state", cx)
                            }),
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
            let metrics = workspace_metrics(workspace);
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
                .child(self.render_detail_row(
                    "Pending",
                    format!(
                        "{} total ({} created, {} updated, {} deleted)",
                        metrics.pending_local_changes,
                        metrics.pending_created,
                        metrics.pending_updated,
                        metrics.pending_deleted
                    ),
                ))
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

    fn render_readiness_check_row(&self, name: &str, status: &str) -> impl IntoElement {
        let colors = self.colors;
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(88.))
                    .child(Label::new(name).text_color(colors.text)),
            )
            .child(self.render_status_badge(status))
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
        let file_for_versions = file.clone();
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
                Button::new(format!("versions-file-{}", file.id))
                    .icon(IconName::Calendar)
                    .ghost()
                    .small()
                    .disabled(self.loading || file.node_type != "file")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.show_file_versions(file_for_versions.clone(), cx)
                    })),
            )
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

    fn render_file_version_row(
        &self,
        version: &FileVersion,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors;
        let pinned = is_file_version_pinned(version);
        let version_for_restore = version.clone();
        let version_for_pin = version.clone();
        let version_for_unpin = version.clone();
        h_flex()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::Calendar)
                    .small()
                    .text_color(colors.accent),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Label::new(file_version_label(version)).text_color(colors.text))
                    .child(
                        Label::new(version.id.as_str())
                            .text_color(colors.muted)
                            .text_size(rems(0.72)),
                    ),
            )
            .child(Label::new(format_bytes(version.size)).text_color(colors.muted))
            .child(
                Label::new(short_hash(&version.sha256))
                    .text_color(colors.muted)
                    .text_size(rems(0.72)),
            )
            .child(
                Label::new(format_optional(version.created_at.as_deref())).text_color(colors.muted),
            )
            .child(self.render_status_badge(if pinned { "pinned" } else { "unpinned" }))
            .child(
                Button::new(format!(
                    "restore-version-{}-{}",
                    version.file_id, version.version
                ))
                .icon(IconName::Undo2)
                .label("Restore")
                .ghost()
                .small()
                .disabled(self.loading)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.restore_file_version(version_for_restore.clone(), cx)
                })),
            )
            .child(
                Button::new(format!(
                    "pin-version-{}-{}",
                    version.file_id, version.version
                ))
                .icon(IconName::Star)
                .label("Pin")
                .ghost()
                .small()
                .disabled(self.loading || pinned)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_file_version_pin(version_for_pin.clone(), true, cx)
                })),
            )
            .child(
                Button::new(format!(
                    "unpin-version-{}-{}",
                    version.file_id, version.version
                ))
                .icon(IconName::Close)
                .label("Unpin")
                .ghost()
                .small()
                .disabled(self.loading || !pinned)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_file_version_pin(version_for_unpin.clone(), false, cx)
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

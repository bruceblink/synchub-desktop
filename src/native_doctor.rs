use crate::client::SyncHubClient;
use crate::models::{CliConfig, WorkspaceSnapshot};
use crate::native_sync::build_sync_plan;
use anyhow::Result;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoctorStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorCheck {
    pub status: DoctorStatus,
    pub name: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn ok(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.status == DoctorStatus::Fail)
    }

    pub fn summary(&self) -> String {
        let warnings = self
            .checks
            .iter()
            .filter(|check| check.status == DoctorStatus::Warn)
            .count();
        if self.ok() {
            format!("Doctor passed with {warnings} warning(s)")
        } else {
            "Doctor found failing checks".to_string()
        }
    }

    pub fn display(&self) -> String {
        self.checks
            .iter()
            .map(|check| {
                let status = match check.status {
                    DoctorStatus::Ok => "OK",
                    DoctorStatus::Warn => "WARN",
                    DoctorStatus::Fail => "FAIL",
                };
                format!("[{status}] {}: {}", check.name, check.detail)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub async fn run_doctor(
    client: &SyncHubClient,
    login: &CliConfig,
    workspace: &WorkspaceSnapshot,
) -> Result<DoctorReport> {
    let mut report = DoctorReport::default();
    let root = workspace.root_path();
    report.checks.push(DoctorCheck {
        status: if root.is_dir() {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Fail
        },
        name: "Workspace",
        detail: root.display().to_string(),
    });
    report.checks.push(DoctorCheck {
        status: if workspace.config.is_some() && workspace.config_error.is_none() {
            DoctorStatus::Ok
        } else {
            DoctorStatus::Fail
        },
        name: "Configuration",
        detail: workspace
            .config_error
            .clone()
            .unwrap_or_else(|| workspace.workspace_config_path().display().to_string()),
    });
    report.checks.push(DoctorCheck {
        status: if login.tokens.access_token.trim().is_empty() {
            DoctorStatus::Fail
        } else {
            DoctorStatus::Ok
        },
        name: "Sign in",
        detail: login.user.email.clone(),
    });

    match client.ready().await {
        Ok(ready) if ready.status.eq_ignore_ascii_case("ready") => {
            report.checks.push(DoctorCheck {
                status: DoctorStatus::Ok,
                name: "Server",
                detail: client.base_url().to_string(),
            })
        }
        Ok(ready) => report.checks.push(DoctorCheck {
            status: DoctorStatus::Fail,
            name: "Server",
            detail: format!("status {}", ready.status),
        }),
        Err(error) => report.checks.push(DoctorCheck {
            status: DoctorStatus::Fail,
            name: "Server",
            detail: error.to_string(),
        }),
    }

    let device_id = workspace.device_id();
    if device_id.is_empty() {
        report.checks.push(DoctorCheck {
            status: DoctorStatus::Warn,
            name: "Device",
            detail: "not registered; Sync Once will register it".to_string(),
        });
    } else {
        match client.list_devices(&login.tokens.access_token, 500).await {
            Ok(devices) if devices.items.iter().any(|device| device.id == device_id) => {
                report.checks.push(DoctorCheck {
                    status: DoctorStatus::Ok,
                    name: "Device",
                    detail: device_id,
                })
            }
            Ok(_) => report.checks.push(DoctorCheck {
                status: DoctorStatus::Fail,
                name: "Device",
                detail: format!("{device_id} is not present on server"),
            }),
            Err(error) => report.checks.push(DoctorCheck {
                status: DoctorStatus::Fail,
                name: "Authorization",
                detail: error.to_string(),
            }),
        }
    }

    match build_sync_plan(workspace) {
        Ok((manifest, plan)) => report.checks.push(DoctorCheck {
            status: if plan.entries.is_empty() {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Warn
            },
            name: "Manifest",
            detail: format!(
                "{} tracked file(s), {}",
                manifest.items.len(),
                plan.summary()
            ),
        }),
        Err(error) => report.checks.push(DoctorCheck {
            status: DoctorStatus::Fail,
            name: "Manifest",
            detail: error.to_string(),
        }),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_fails_only_for_failed_checks() {
        let report = DoctorReport {
            checks: vec![
                DoctorCheck {
                    status: DoctorStatus::Ok,
                    name: "one",
                    detail: "ok".into(),
                },
                DoctorCheck {
                    status: DoctorStatus::Warn,
                    name: "two",
                    detail: "warning".into(),
                },
            ],
        };
        assert!(report.ok());
        assert_eq!(report.summary(), "Doctor passed with 1 warning(s)");
        assert!(report.display().contains("[WARN] two: warning"));
    }
}

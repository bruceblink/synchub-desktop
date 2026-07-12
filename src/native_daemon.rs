use crate::client::{SyncHubClient, refresh_cli_config_if_needed};
use crate::config::{
    load_cli_config, load_workspace_snapshot, read_optional_json, save_cli_config, write_json,
};
use crate::models::{SyncAgentControl, SyncAgentState, WorkspaceRegistryEntry};
use crate::native_sync::execute_sync_once;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;

const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

pub fn start_daemon(entry: WorkspaceRegistryEntry, config_path: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_daemon(entry, config_path, DEFAULT_INTERVAL).await;
    })
}

pub fn set_paused(root: &Path, paused: bool) -> Result<SyncAgentControl> {
    let control = SyncAgentControl {
        version: 1,
        paused,
        updated_at: Some(now()),
    };
    write_json(&control_path(root), &control)?;
    Ok(control)
}

pub fn reset_state(root: &Path) -> Result<()> {
    let path = state_path(root);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

async fn run_daemon(entry: WorkspaceRegistryEntry, config_path: PathBuf, interval: Duration) {
    let root = PathBuf::from(&entry.root);
    loop {
        let paused = read_optional_json::<SyncAgentControl>(&control_path(&root))
            .ok()
            .flatten()
            .is_some_and(|control| control.paused);
        if paused {
            update_state(&root, |state| {
                state.status = "paused".to_string();
            });
        } else {
            run_cycle(&entry, &config_path).await;
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_cycle(entry: &WorkspaceRegistryEntry, config_path: &Path) {
    let root = PathBuf::from(&entry.root);
    update_state(&root, |state| state.status = "running".to_string());
    let result = async {
        let mut login = load_cli_config(config_path)?.context("sign in before starting sync")?;
        if refresh_cli_config_if_needed(&mut login).await? {
            save_cli_config(config_path, &login)?;
        }
        let workspace = load_workspace_snapshot(entry.clone());
        let server = workspace.server_url(&login.server_url);
        let client = SyncHubClient::new(server)?;
        execute_sync_once(&client, &login.tokens.access_token, &workspace).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    update_state(&root, |state| {
        state.cycles_run += 1;
        match result {
            Ok(()) => {
                state.status = "idle".to_string();
                state.consecutive_failures = 0;
                state.last_success_at = Some(now());
                state.last_error.clear();
            }
            Err(error) => {
                state.status = "error".to_string();
                state.consecutive_failures += 1;
                state.last_failure_at = Some(now());
                state.last_error = format!("{error:#}");
            }
        }
    });
}

fn update_state(root: &Path, update: impl FnOnce(&mut SyncAgentState)) {
    let path = state_path(root);
    let mut state = read_optional_json::<SyncAgentState>(&path)
        .ok()
        .flatten()
        .unwrap_or_else(|| SyncAgentState {
            version: 1,
            root: root.display().to_string(),
            ..SyncAgentState::default()
        });
    update(&mut state);
    state.updated_at = Some(now());
    let _ = write_json(&path, &state);
}

fn state_path(root: &Path) -> PathBuf {
    root.join(".synchub").join("daemon-state.json")
}

fn control_path(root: &Path) -> PathBuf {
    root.join(".synchub").join("daemon-control.json")
}

fn now() -> String {
    crate::app::time::rfc3339_from_system_time(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_resume_and_reset_are_managed_without_cli() {
        let root = tempfile::tempdir().unwrap();
        assert!(set_paused(root.path(), true).unwrap().paused);
        assert!(!set_paused(root.path(), false).unwrap().paused);
        update_state(root.path(), |state| state.status = "error".into());
        assert!(state_path(root.path()).is_file());
        reset_state(root.path()).unwrap();
        assert!(!state_path(root.path()).exists());
    }
}

use crate::models::{
    CliConfig, Manifest, SyncAgentControl, SyncAgentState, WorkspaceConfig, WorkspaceRegistry,
    WorkspaceRegistryEntry, WorkspaceSnapshot,
};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DesktopSettings {
    pub server_url: String,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:8765".to_string(),
        }
    }
}

pub fn desktop_config_dir() -> Result<PathBuf> {
    if let Some(dirs) = ProjectDirs::from("app", "likanug", "SyncHubDesktop") {
        return Ok(dirs.config_dir().to_path_buf());
    }
    Ok(std::env::current_dir()?.join(".synchub-desktop"))
}

pub fn settings_path() -> Result<PathBuf> {
    Ok(desktop_config_dir()?.join("settings.json"))
}

pub fn load_settings() -> DesktopSettings {
    settings_path()
        .ok()
        .and_then(|path| read_optional_json::<DesktopSettings>(&path).ok().flatten())
        .unwrap_or_default()
}

pub fn save_settings(settings: &DesktopSettings) -> Result<()> {
    write_json(&settings_path()?, settings)
}

pub fn default_cli_config_path() -> PathBuf {
    if let Ok(value) = std::env::var("SYNCHUB_CONFIG") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.config_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(".synchub"))
        .join("SyncHub")
        .join("config.json")
}

pub fn default_workspace_registry_path(config_path: &Path) -> PathBuf {
    if let Ok(value) = std::env::var("SYNCHUB_WORKSPACES") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    config_path
        .parent()
        .map(|path| path.join("workspaces.json"))
        .unwrap_or_else(|| PathBuf::from(".synchub").join("workspaces.json"))
}

pub fn load_cli_config(config_path: &Path) -> Result<Option<CliConfig>> {
    read_optional_json(config_path)
}

pub fn save_cli_config(config_path: &Path, config: &CliConfig) -> Result<()> {
    write_json(config_path, config)
}

pub fn remove_cli_config(config_path: &Path) -> Result<()> {
    match fs::remove_file(config_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove {}", config_path.display())),
    }
}

pub fn load_workspace_registry(path: &Path) -> Result<WorkspaceRegistry> {
    Ok(
        read_optional_json::<WorkspaceRegistry>(path)?.unwrap_or_else(|| WorkspaceRegistry {
            version: 1,
            ..WorkspaceRegistry::default()
        }),
    )
}

pub fn load_workspace_snapshots(registry_path: &Path) -> Result<Vec<WorkspaceSnapshot>> {
    let registry = load_workspace_registry(registry_path)?;
    let mut snapshots = Vec::new();
    for entry in registry.workspaces {
        snapshots.push(load_workspace_snapshot(entry));
    }
    Ok(snapshots)
}

pub fn load_workspace_snapshot(entry: WorkspaceRegistryEntry) -> WorkspaceSnapshot {
    let mut snapshot = WorkspaceSnapshot {
        entry,
        ..WorkspaceSnapshot::default()
    };

    let config_path = if snapshot.entry.workspace_config_path.is_empty() {
        snapshot.root_path().join(".synchub").join("workspace.json")
    } else {
        PathBuf::from(&snapshot.entry.workspace_config_path)
    };

    match read_optional_json::<WorkspaceConfig>(&config_path) {
        Ok(config) => snapshot.config = config,
        Err(error) => snapshot.config_error = Some(error.to_string()),
    }

    let root = snapshot.root_path();
    let manifest_path = root.join(".synchub").join("manifest.json");
    if let Ok(manifest) = read_optional_json::<Manifest>(&manifest_path) {
        snapshot.manifest = manifest;
    }
    if let Ok(state) =
        read_optional_json::<SyncAgentState>(&root.join(".synchub").join("daemon-state.json"))
    {
        snapshot.daemon_state = state;
    }
    if let Ok(control) =
        read_optional_json::<SyncAgentControl>(&root.join(".synchub").join("daemon-control.json"))
    {
        snapshot.daemon_control = control;
    }
    snapshot.trash_entries = count_trash_entries(&root.join(".synchub").join("trash"));

    snapshot
}

fn count_trash_entries(path: &Path) -> usize {
    let Ok(batches) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for batch in batches.flatten() {
        let batch_path = batch.path();
        if !batch_path.is_dir() {
            continue;
        }
        total += count_files_recursively(&batch_path);
    }
    total
}

fn count_files_recursively(path: &Path) -> usize {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            total += count_files_recursively(&entry_path);
        } else {
            total += 1;
        }
    }
    total
}

pub fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let value = serde_json::from_str(&raw).with_context(|| format!("decode {}", path.display()))?;
    Ok(Some(value))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(value)?;
    fs::write(path, [raw.as_slice(), b"\n"].concat())
        .with_context(|| format!("write {}", path.display()))
}

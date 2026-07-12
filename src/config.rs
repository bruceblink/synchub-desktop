use crate::models::{
    CliConfig, Manifest, SyncAgentControl, SyncAgentState, WorkspaceConfig, WorkspaceRegistry,
    WorkspaceRegistryEntry, WorkspaceSnapshot, pending_manifest_changes,
};
use anyhow::{Context, Result, bail};
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

pub fn load_settings_with_legacy_cli(config_path: &Path) -> DesktopSettings {
    settings_path()
        .ok()
        .map(|path| load_settings_from_paths(&path, config_path))
        .unwrap_or_default()
}

pub fn load_settings_from_paths(settings_path: &Path, config_path: &Path) -> DesktopSettings {
    if let Ok(Some(settings)) = read_optional_json::<DesktopSettings>(settings_path) {
        return settings;
    }
    load_cli_config(config_path)
        .ok()
        .flatten()
        .map(|config| DesktopSettings {
            server_url: config.server_url,
        })
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

pub fn save_workspace_config(path: &Path, config: &WorkspaceConfig) -> Result<()> {
    write_json(path, config)
}

pub fn update_cli_server_url(config_path: &Path, server_url: &str) -> Result<Option<CliConfig>> {
    let Some(mut config) = load_cli_config(config_path)? else {
        return Ok(None);
    };
    config.server_url = server_url.to_string();
    save_cli_config(config_path, &config)?;
    Ok(Some(config))
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

pub fn initialize_workspaces(
    roots: &[String],
    remote_root: &str,
    login: &CliConfig,
    registry_path: &Path,
    legacy_config_path: &Path,
) -> Result<Vec<WorkspaceRegistryEntry>> {
    if roots.is_empty() {
        bail!("workspace path is required");
    }
    if login.user.id.trim().is_empty() {
        bail!("sign in before initializing a workspace");
    }
    let now = crate::app::time::rfc3339_from_system_time(std::time::SystemTime::now());
    let mut prepared = Vec::with_capacity(roots.len());
    for root in roots {
        let root = clean_existing_path(Path::new(root.trim()))
            .with_context(|| format!("resolve workspace root {root}"))?;
        if !root.is_dir() {
            bail!("workspace root is not a directory: {}", root.display());
        }
        if prepared
            .iter()
            .any(|(existing, _, _): &(PathBuf, String, PathBuf)| same_path(existing, &root))
        {
            bail!("duplicate workspace path: {}", root.display());
        }
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .context("workspace root must have a directory name")?;
        let remote_path = join_remote_root(remote_root, name);
        if prepared
            .iter()
            .any(|(_, existing, _)| existing == &remote_path)
        {
            bail!("remote path is already used: {remote_path}");
        }
        let workspace_path = root.join(".synchub").join("workspace.json");
        prepared.push((root, remote_path, workspace_path));
    }

    let mut registry = load_workspace_registry(registry_path)?;
    registry.version = 1;
    registry.updated_at = Some(now.clone());
    let mut initialized = Vec::with_capacity(prepared.len());
    for (root, remote_path, workspace_path) in prepared {
        let workspace = WorkspaceConfig {
            version: 1,
            root: root.display().to_string(),
            remote_path: remote_path.clone(),
            server_url: login.server_url.clone(),
            user_id: login.user.id.clone(),
            user_email: login.user.email.clone(),
            created_at: Some(now.clone()),
            updated_at: Some(now.clone()),
            ..WorkspaceConfig::default()
        };
        write_json(&workspace_path, &workspace)?;
        let entry = WorkspaceRegistryEntry {
            root: workspace.root.clone(),
            workspace_config_path: workspace_path.display().to_string(),
            config_path: legacy_config_path.display().to_string(),
            remote_path,
            server_url: login.server_url.clone(),
            user_id: login.user.id.clone(),
            user_email: login.user.email.clone(),
            updated_at: Some(now.clone()),
        };
        registry.workspaces.retain(|existing| {
            !same_path(Path::new(&existing.root), Path::new(&entry.root))
                && !same_path(
                    Path::new(&existing.workspace_config_path),
                    Path::new(&entry.workspace_config_path),
                )
        });
        registry.workspaces.push(entry.clone());
        initialized.push(entry);
    }
    registry
        .workspaces
        .sort_by(|left, right| left.root.cmp(&right.root));
    write_json(registry_path, &registry)?;
    Ok(initialized)
}

pub fn remove_workspace_registration(registry_path: &Path, root: &Path) -> Result<bool> {
    let mut registry = load_workspace_registry(registry_path)?;
    let before = registry.workspaces.len();
    registry
        .workspaces
        .retain(|entry| !same_path(Path::new(&entry.root), root));
    if registry.workspaces.len() == before {
        return Ok(false);
    }
    registry.updated_at = Some(crate::app::time::rfc3339_from_system_time(
        std::time::SystemTime::now(),
    ));
    write_json(registry_path, &registry)?;
    Ok(true)
}

pub fn prune_workspace_registrations(registry_path: &Path) -> Result<usize> {
    let mut registry = load_workspace_registry(registry_path)?;
    let before = registry.workspaces.len();
    registry.workspaces.retain(|entry| {
        Path::new(&entry.root).is_dir() && Path::new(&entry.workspace_config_path).is_file()
    });
    let removed = before - registry.workspaces.len();
    if removed > 0 {
        registry.updated_at = Some(crate::app::time::rfc3339_from_system_time(
            std::time::SystemTime::now(),
        ));
        write_json(registry_path, &registry)?;
    }
    Ok(removed)
}

fn join_remote_root(remote_root: &str, name: &str) -> String {
    let root = normalize_remote_path(remote_root);
    if root == "/" {
        format!("/{name}")
    } else {
        format!("{root}/{name}")
    }
}

fn normalize_remote_path(value: &str) -> String {
    let mut parts = Vec::new();
    let normalized = value.trim().replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part.to_string()),
        }
    }
    format!("/{}", parts.join("/"))
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = comparable_path(left);
    let right = comparable_path(right);
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn clean_existing_path(path: &Path) -> Result<PathBuf> {
    Ok(strip_windows_verbatim(fs::canonicalize(path)?))
}

fn comparable_path(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .map(strip_windows_verbatim)
        .unwrap_or_else(|_| strip_windows_verbatim(path.to_path_buf()))
}

fn strip_windows_verbatim(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}

pub fn update_workspace_server_urls(registry_path: &Path, server_url: &str) -> Result<usize> {
    let server_url = server_url.trim();
    if server_url.is_empty() {
        return Ok(0);
    }

    let mut registry = load_workspace_registry(registry_path)?;
    let mut updated = 0;
    for entry in &mut registry.workspaces {
        let config_path = if entry.workspace_config_path.trim().is_empty() {
            PathBuf::from(&entry.root)
                .join(".synchub")
                .join("workspace.json")
        } else {
            PathBuf::from(&entry.workspace_config_path)
        };

        if let Some(mut config) = read_optional_json::<WorkspaceConfig>(&config_path)? {
            if config.server_url != server_url {
                config.server_url = server_url.to_string();
                write_json(&config_path, &config)?;
                updated += 1;
            }
        }
        entry.server_url = server_url.to_string();
    }

    if !registry.workspaces.is_empty() {
        write_json(registry_path, &registry)?;
    }
    Ok(updated)
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
    snapshot.pending_changes = pending_manifest_changes(&snapshot);
    if let Ok(state) =
        read_optional_json::<SyncAgentState>(&root.join(".synchub").join("daemon-state.json"))
    {
        snapshot.daemon_state = state.map(clear_legacy_cli_error);
    }
    if let Ok(control) =
        read_optional_json::<SyncAgentControl>(&root.join(".synchub").join("daemon-control.json"))
    {
        snapshot.daemon_control = control;
    }
    snapshot.trash_entries = count_trash_entries(&root.join(".synchub").join("trash"));

    snapshot
}

fn clear_legacy_cli_error(mut state: SyncAgentState) -> SyncAgentState {
    if state
        .last_error
        .to_ascii_lowercase()
        .contains("synchub-cli")
    {
        state.last_error.clear();
        state.last_failure_at = None;
        state.consecutive_failures = 0;
        if state.status == "error" {
            state.status = "idle".to_string();
        }
    }
    state
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_snapshot_hides_retired_cli_daemon_errors() {
        let state = SyncAgentState {
            status: "error".to_string(),
            consecutive_failures: 3,
            last_failure_at: Some("2026-07-12T00:00:00Z".to_string()),
            last_error: "not logged in; run synchub-cli login first".to_string(),
            ..SyncAgentState::default()
        };

        let state = clear_legacy_cli_error(state);

        assert_eq!(state.status, "idle");
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.last_failure_at, None);
        assert!(state.last_error.is_empty());
    }

    #[test]
    fn workspace_snapshot_preserves_native_daemon_errors() {
        let state = SyncAgentState {
            status: "error".to_string(),
            consecutive_failures: 1,
            last_error: "refresh token is invalid".to_string(),
            ..SyncAgentState::default()
        };

        assert_eq!(clear_legacy_cli_error(state.clone()), state);
    }
}

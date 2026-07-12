use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ApiStatus {
    pub status: String,
    #[serde(default)]
    pub checks: BTreeMap<String, StatusCheck>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StatusCheck {
    pub status: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct VersionInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

impl TokenPair {
    pub fn access_token_expires_at(&self, now: SystemTime) -> SystemTime {
        now + Duration::from_secs(self.expires_in.max(0) as u64)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LoginData {
    pub user: User,
    pub tokens: TokenPair,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ApiEnvelope<T> {
    pub code: serde_json::Value,
    pub message: String,
    pub data: Option<T>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CliConfig {
    pub server_url: String,
    pub user: User,
    pub tokens: TokenPair,
    pub access_token_expires_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceRegistry {
    pub version: i32,
    pub updated_at: Option<String>,
    pub workspaces: Vec<WorkspaceRegistryEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceRegistryEntry {
    pub root: String,
    pub workspace_config_path: String,
    pub config_path: String,
    pub remote_path: String,
    pub server_url: String,
    pub user_id: String,
    pub user_email: String,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkspaceConfig {
    pub version: i32,
    pub root: String,
    pub remote_path: String,
    pub server_url: String,
    pub user_id: String,
    pub user_email: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub device_platform: Option<String>,
    pub last_applied_change_id: Option<i64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Manifest {
    pub version: i32,
    pub root: String,
    pub remote_path: String,
    pub generated_at: Option<String>,
    pub items: Vec<ManifestEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ManifestEntry {
    pub path: String,
    pub relative_path: String,
    pub size: i64,
    pub sha256: String,
    pub mtime: Option<String>,
    pub remote_version: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SyncAgentState {
    pub version: i32,
    pub root: String,
    pub status: String,
    pub cycles_run: i32,
    pub consecutive_failures: i32,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error: String,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SyncAgentControl {
    pub version: i32,
    pub paused: bool,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileListData {
    pub items: Vec<FileNode>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub size: i64,
    pub sha256: Option<String>,
    pub version: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

pub fn file_belongs_to_remote_root(path: &str, remote_root: &str) -> bool {
    let path = path.trim().trim_end_matches('/');
    let root = remote_root.trim().trim_end_matches('/');
    root.is_empty() || root == "/" || path == root || path.starts_with(&format!("{root}/"))
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileVersionListData {
    pub items: Vec<FileVersion>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FileVersion {
    pub id: String,
    pub file_id: String,
    pub version: i64,
    pub size: i64,
    pub sha256: String,
    pub pinned_at: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RestoreFileVersionData {
    pub file: FileNode,
    pub change_id: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SyncConflictListData {
    pub items: Vec<SyncConflict>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DeviceListData {
    pub items: Vec<Device>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub last_seen_at: Option<String>,
    pub last_applied_change_id: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SyncConflict {
    pub id: String,
    pub file_id: Option<String>,
    pub path: String,
    pub local_version: Option<i64>,
    pub remote_version: Option<i64>,
    pub resolution: String,
    pub created_at: Option<String>,
    pub resolved_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TrashEntry {
    pub batch: String,
    pub path: String,
    pub size: i64,
    pub is_dir: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SyncTrashSnapshot {
    pub items: Vec<TrashEntry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceSnapshot {
    pub entry: WorkspaceRegistryEntry,
    pub config: Option<WorkspaceConfig>,
    pub manifest: Option<Manifest>,
    pub pending_changes: PendingManifestChanges,
    pub daemon_state: Option<SyncAgentState>,
    pub daemon_control: Option<SyncAgentControl>,
    pub trash_entries: usize,
    pub config_error: Option<String>,
}

impl WorkspaceSnapshot {
    pub fn root_path(&self) -> PathBuf {
        PathBuf::from(if self.entry.root.is_empty() {
            self.config
                .as_ref()
                .map(|config| config.root.as_str())
                .unwrap_or("")
        } else {
            self.entry.root.as_str()
        })
    }

    pub fn display_name(&self) -> String {
        let root = self.root_path();
        root.file_name()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.entry.remote_path.clone())
    }

    pub fn remote_path(&self) -> String {
        self.config
            .as_ref()
            .map(|config| config.remote_path.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.entry.remote_path.clone())
    }

    pub fn device_id(&self) -> String {
        self.config
            .as_ref()
            .and_then(|config| config.device_id.clone())
            .unwrap_or_default()
    }

    pub fn server_url(&self, fallback: &str) -> String {
        self.config
            .as_ref()
            .map(|config| config.server_url.clone())
            .filter(|value| !value.is_empty())
            .or_else(|| (!self.entry.server_url.is_empty()).then(|| self.entry.server_url.clone()))
            .unwrap_or_else(|| fallback.to_string())
    }

    pub fn workspace_config_path(&self) -> PathBuf {
        if self.entry.workspace_config_path.trim().is_empty() {
            self.root_path().join(".synchub").join("workspace.json")
        } else {
            PathBuf::from(&self.entry.workspace_config_path)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceMetrics {
    pub manifest_files: usize,
    pub remote_tracked: usize,
    pub local_only: usize,
    pub pending_local_changes: usize,
    pub pending_created: usize,
    pub pending_updated: usize,
    pub pending_deleted: usize,
    pub trash_entries: usize,
    pub daemon_status: String,
}

pub fn workspace_metrics(snapshot: &WorkspaceSnapshot) -> WorkspaceMetrics {
    let mut metrics = WorkspaceMetrics {
        trash_entries: snapshot.trash_entries,
        daemon_status: snapshot
            .daemon_state
            .as_ref()
            .map(|state| state.status.clone())
            .filter(|status| !status.is_empty())
            .unwrap_or_else(|| "not run".to_string()),
        ..WorkspaceMetrics::default()
    };

    if let Some(manifest) = &snapshot.manifest {
        metrics.manifest_files = manifest.items.len();
        for item in &manifest.items {
            if item.remote_version.is_some() {
                metrics.remote_tracked += 1;
            } else {
                metrics.local_only += 1;
            }
        }
    }
    let pending = &snapshot.pending_changes;
    metrics.pending_local_changes = pending.total();
    metrics.pending_created = pending.created;
    metrics.pending_updated = pending.updated;
    metrics.pending_deleted = pending.deleted;

    metrics
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PendingManifestChanges {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
}

impl PendingManifestChanges {
    pub fn total(&self) -> usize {
        self.created + self.updated + self.deleted
    }
}

pub fn pending_manifest_changes(snapshot: &WorkspaceSnapshot) -> PendingManifestChanges {
    let Some(manifest) = &snapshot.manifest else {
        return PendingManifestChanges::default();
    };
    let root = snapshot.root_path();
    if root.as_os_str().is_empty() {
        return PendingManifestChanges::default();
    }

    let previous = manifest
        .items
        .iter()
        .filter_map(|item| {
            normalized_relative_path(item).map(|relative| {
                (
                    relative,
                    ManifestFileFingerprint {
                        size: item.size,
                        sha256: item.sha256.clone(),
                    },
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let current = scan_workspace_files(&root);

    let previous_paths = previous.keys().cloned().collect::<HashSet<_>>();
    let current_paths = current.keys().cloned().collect::<HashSet<_>>();

    let created = current_paths.difference(&previous_paths).count();
    let deleted = previous_paths.difference(&current_paths).count();
    let updated = current_paths
        .intersection(&previous_paths)
        .filter(|path| previous.get(*path) != current.get(*path))
        .count();

    PendingManifestChanges {
        created,
        updated,
        deleted,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestFileFingerprint {
    size: i64,
    sha256: String,
}

fn normalized_relative_path(item: &ManifestEntry) -> Option<String> {
    let value = item.relative_path.trim().replace('\\', "/");
    (!value.is_empty()).then_some(value)
}

fn scan_workspace_files(root: &PathBuf) -> HashMap<String, ManifestFileFingerprint> {
    let mut files = HashMap::new();
    scan_workspace_files_inner(root, root, &mut files);
    files
}

fn scan_workspace_files_inner(
    root: &PathBuf,
    current: &PathBuf,
    files: &mut HashMap<String, ManifestFileFingerprint>,
) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_str() == Some(".synchub") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            scan_workspace_files_inner(root, &path, files);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        let sha256 = file_sha256_hex(&path).unwrap_or_default();
        files.insert(
            relative,
            ManifestFileFingerprint {
                size: metadata.len() as i64,
                sha256,
            },
        );
    }
}

fn file_sha256_hex(path: &PathBuf) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

pub fn format_bytes(size: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size.max(0) as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", size.max(0), units[unit])
    } else {
        format!("{:.1} {}", value, units[unit])
    }
}

pub fn file_version_label(version: &FileVersion) -> String {
    format!("v{}", version.version)
}

pub fn is_file_version_pinned(version: &FileVersion) -> bool {
    version
        .pinned_at
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

pub fn compose_remote_directory_path(input: &str, workspace_remote: &str) -> Option<String> {
    let input = input
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .replace('\\', "/");
    if input.is_empty() || input.contains('\0') {
        return None;
    }

    let path = if input.starts_with('/') {
        input
    } else {
        let base = workspace_remote.trim().replace('\\', "/");
        if base.is_empty() || base == "/" {
            format!("/{}", input.trim_start_matches('/'))
        } else {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                input.trim_start_matches('/')
            )
        }
    };
    let mut parts = Vec::new();
    for part in path.split('/') {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

pub fn conflict_resolution_label(resolution: &str) -> &'static str {
    match resolution {
        "pending" => "pending",
        "keep_local" => "keep local",
        "keep_remote" => "keep remote",
        "keep_both" => "keep both",
        _ => "unknown",
    }
}

pub fn is_current_device(device: &Device, snapshot: &WorkspaceSnapshot) -> bool {
    let current = snapshot.device_id();
    !current.trim().is_empty() && device.id == current
}

pub fn is_success_code(code: &serde_json::Value) -> bool {
    match code {
        serde_json::Value::Null => true,
        serde_json::Value::Number(value) => value.as_i64() == Some(0),
        serde_json::Value::String(value) => value.is_empty() || value == "0",
        _ => false,
    }
}

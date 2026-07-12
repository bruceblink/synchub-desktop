use crate::client::SyncHubClient;
use crate::models::{Manifest, ManifestEntry, WorkspaceSnapshot};
use crate::native_manifest::{scan_current_manifest, write_manifest};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncPlanAction {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPlanEntry {
    pub action: SyncPlanAction,
    pub relative_path: String,
    pub remote_path: String,
    pub size: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncPlan {
    pub entries: Vec<SyncPlanEntry>,
}

impl SyncPlan {
    pub fn created(&self) -> usize {
        self.count(SyncPlanAction::Create)
    }

    pub fn updated(&self) -> usize {
        self.count(SyncPlanAction::Update)
    }

    pub fn deleted(&self) -> usize {
        self.count(SyncPlanAction::Delete)
    }

    pub fn summary(&self) -> String {
        format!(
            "{} change(s): {} created, {} updated, {} deleted",
            self.entries.len(),
            self.created(),
            self.updated(),
            self.deleted()
        )
    }

    pub fn display(&self) -> String {
        if self.entries.is_empty() {
            return "No local changes".to_string();
        }
        self.entries
            .iter()
            .map(|entry| {
                let action = match entry.action {
                    SyncPlanAction::Create => "create",
                    SyncPlanAction::Update => "update",
                    SyncPlanAction::Delete => "delete",
                };
                format!("{action} {}", entry.relative_path)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn count(&self, action: SyncPlanAction) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action == action)
            .count()
    }
}

pub fn build_sync_plan(workspace: &WorkspaceSnapshot) -> Result<(Manifest, SyncPlan)> {
    let current = scan_current_manifest(workspace)?;
    let previous = workspace.manifest.clone().unwrap_or_default();
    Ok((current.clone(), compare_manifests(&previous, &current)))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PushResult {
    pub uploaded: usize,
    pub deleted: usize,
}

impl PushResult {
    pub fn summary(&self) -> String {
        format!(
            "pushed {} file(s), deleted {} file(s)",
            self.uploaded, self.deleted
        )
    }
}

pub async fn execute_push(
    client: &SyncHubClient,
    access_token: &str,
    workspace: &WorkspaceSnapshot,
) -> Result<PushResult> {
    let (mut current, plan) = build_sync_plan(workspace)?;
    let previous = workspace.manifest.clone().unwrap_or_default();
    let previous_by_path = entries_by_path(&previous);
    let root = workspace.root_path();
    let device_id = workspace.device_id();
    let device_id = (!device_id.trim().is_empty()).then_some(device_id.as_str());
    let mut ensured_directories = HashSet::new();
    let mut result = PushResult::default();

    for entry in &plan.entries {
        match entry.action {
            SyncPlanAction::Create | SyncPlanAction::Update => {
                ensure_remote_directories(
                    client,
                    access_token,
                    device_id,
                    &entry.remote_path,
                    &mut ensured_directories,
                )
                .await?;
                let item = current
                    .items
                    .iter_mut()
                    .find(|item| item.relative_path.replace('\\', "/") == entry.relative_path)
                    .context("planned upload is missing from current manifest")?;
                let base_version = previous_by_path
                    .get(&entry.relative_path)
                    .and_then(|item| item.remote_version);
                let version = upload_manifest_entry(
                    client,
                    access_token,
                    device_id,
                    &root,
                    item,
                    base_version,
                )
                .await?;
                item.remote_version = Some(version);
                result.uploaded += 1;
            }
            SyncPlanAction::Delete => {
                let old = previous_by_path
                    .get(&entry.relative_path)
                    .context("planned delete is missing from previous manifest")?;
                let node = client
                    .get_file_by_path(access_token, &old.path)
                    .await
                    .with_context(|| format!("find remote file {}", old.path))?;
                client
                    .delete_file_versioned(access_token, &node.id, device_id, old.remote_version)
                    .await
                    .with_context(|| format!("delete remote file {}", old.path))?;
                result.deleted += 1;
            }
        }
    }

    let manifest_path = root.join(".synchub").join("manifest.json");
    write_manifest(&manifest_path, &current)?;
    Ok(result)
}

async fn upload_manifest_entry(
    client: &SyncHubClient,
    access_token: &str,
    device_id: Option<&str>,
    root: &Path,
    item: &ManifestEntry,
    base_version: Option<i64>,
) -> Result<i64> {
    let key = format!(
        "{}:{}:{}",
        item.path,
        item.sha256,
        base_version.unwrap_or(0)
    );
    let session = client
        .init_upload(
            access_token,
            &item.path,
            item.size,
            &item.sha256,
            base_version,
            device_id,
            &key,
        )
        .await?;
    if session.chunk_size <= 0 {
        bail!("server returned invalid upload chunk size");
    }
    let chunk_size = usize::try_from(session.chunk_size)
        .context("server returned unsupported upload chunk size")?;
    let local_path = safe_local_path(root, &item.relative_path)?;
    let mut file = tokio::fs::File::open(&local_path)
        .await
        .with_context(|| format!("open {}", local_path.display()))?;
    let mut index = 0_i32;
    let mut buffer = vec![0_u8; chunk_size];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 && index > 0 {
            break;
        }
        let content = buffer[..read].to_vec();
        let hash = format!("{:x}", Sha256::digest(&content));
        client
            .put_upload_chunk(access_token, &session.upload_id, index, content, &hash)
            .await?;
        index += 1;
        if read == 0 || read < buffer.len() {
            break;
        }
    }
    Ok(client
        .commit_upload(access_token, &session.upload_id)
        .await?
        .version)
}

async fn ensure_remote_directories(
    client: &SyncHubClient,
    access_token: &str,
    device_id: Option<&str>,
    file_path: &str,
    ensured: &mut HashSet<String>,
) -> Result<()> {
    for directory in remote_parent_directories(file_path)? {
        if !ensured.insert(directory.clone()) {
            continue;
        }
        match client.get_file_by_path(access_token, &directory).await {
            Ok(node) if node.node_type == "directory" => continue,
            Ok(_) => bail!("remote parent is not a directory: {directory}"),
            Err(_) => {
                client
                    .create_directory(access_token, &directory, device_id)
                    .await
                    .with_context(|| format!("create remote directory {directory}"))?;
            }
        }
    }
    Ok(())
}

fn remote_parent_directories(file_path: &str) -> Result<Vec<String>> {
    let normalized = file_path.trim().replace('\\', "/");
    if !normalized.starts_with('/') || normalized.split('/').any(|part| part == "..") {
        bail!("invalid remote file path: {file_path}");
    }
    let parts = normalized
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut directories = Vec::new();
    for index in 1..parts.len() {
        directories.push(format!("/{}", parts[..index].join("/")));
    }
    Ok(directories)
}

fn safe_local_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = relative.replace('\\', "/");
    if relative.is_empty()
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("invalid local manifest path: {relative}");
    }
    Ok(root.join(relative))
}

fn compare_manifests(previous: &Manifest, current: &Manifest) -> SyncPlan {
    let previous = entries_by_path(previous);
    let current = entries_by_path(current);
    let mut entries = Vec::new();

    for (path, item) in &current {
        match previous.get(path) {
            None => entries.push(plan_entry(SyncPlanAction::Create, item)),
            Some(old) if old.size != item.size || old.sha256 != item.sha256 => {
                entries.push(plan_entry(SyncPlanAction::Update, item));
            }
            _ => {}
        }
    }
    for (path, item) in &previous {
        if !current.contains_key(path) {
            entries.push(plan_entry(SyncPlanAction::Delete, item));
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    SyncPlan { entries }
}

fn entries_by_path(manifest: &Manifest) -> BTreeMap<String, &ManifestEntry> {
    manifest
        .items
        .iter()
        .filter_map(|item| {
            let path = item.relative_path.trim().replace('\\', "/");
            (!path.is_empty()).then_some((path, item))
        })
        .collect()
}

fn plan_entry(action: SyncPlanAction, item: &ManifestEntry) -> SyncPlanEntry {
    SyncPlanEntry {
        action,
        relative_path: item.relative_path.replace('\\', "/"),
        remote_path: item.path.clone(),
        size: item.size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WorkspaceRegistryEntry;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn plans_created_updated_and_deleted_files_without_changing_baseline() {
        let root = std::env::temp_dir().join(format!(
            "synchub-native-sync-plan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".synchub")).unwrap();
        fs::write(root.join("created.txt"), b"created").unwrap();
        fs::write(root.join("updated.txt"), b"new").unwrap();
        let baseline_path = root.join(".synchub/manifest.json");
        fs::write(&baseline_path, b"baseline remains unchanged").unwrap();
        let workspace = WorkspaceSnapshot {
            entry: WorkspaceRegistryEntry {
                root: root.display().to_string(),
                remote_path: "/workspace".into(),
                ..Default::default()
            },
            manifest: Some(Manifest {
                items: vec![
                    ManifestEntry {
                        relative_path: "updated.txt".into(),
                        path: "/workspace/updated.txt".into(),
                        size: 3,
                        sha256: "old".into(),
                        ..Default::default()
                    },
                    ManifestEntry {
                        relative_path: "deleted.txt".into(),
                        path: "/workspace/deleted.txt".into(),
                        size: 7,
                        sha256: "old".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };

        let (_, plan) = build_sync_plan(&workspace).unwrap();
        assert_eq!((plan.created(), plan.updated(), plan.deleted()), (1, 1, 1));
        assert_eq!(
            fs::read_to_string(baseline_path).unwrap(),
            "baseline remains unchanged"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn push_uploads_chunks_and_saves_committed_version() {
        let root = test_root("push-success");
        let workspace = new_workspace(&root);
        fs::create_dir_all(root.join(".synchub")).unwrap();
        fs::write(root.join("a.txt"), b"hello").unwrap();
        let (server, requests) = upload_server(false).await;

        let result = execute_push(&SyncHubClient::new(server).unwrap(), "token", &workspace)
            .await
            .unwrap();
        assert_eq!(
            result,
            PushResult {
                uploaded: 1,
                deleted: 0
            }
        );
        let manifest: Manifest =
            serde_json::from_str(&fs::read_to_string(root.join(".synchub/manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest.items[0].remote_version, Some(7));
        assert_eq!(
            requests.await.unwrap(),
            vec![
                "GET /api/v1/files/by-path?path=%2Fworkspace",
                "POST /api/v1/files/directories",
                "POST /api/v1/uploads",
                "PUT /api/v1/uploads/upl_1/chunks/0",
                "POST /api/v1/uploads/upl_1/commit",
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_push_preserves_manifest_baseline() {
        let root = test_root("push-failure");
        fs::create_dir_all(root.join(".synchub")).unwrap();
        fs::write(root.join("a.txt"), b"hello").unwrap();
        let baseline = r#"{"version":1,"root":"baseline","remote_path":"/workspace","items":[]}"#;
        fs::write(root.join(".synchub/manifest.json"), baseline).unwrap();
        let workspace = new_workspace(&root);
        let (server, requests) = upload_server(true).await;

        assert!(
            execute_push(&SyncHubClient::new(server).unwrap(), "token", &workspace)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join(".synchub/manifest.json")).unwrap(),
            baseline
        );
        let _ = requests.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn push_deletes_remote_file_with_baseline_version() {
        let root = test_root("push-delete");
        fs::create_dir_all(root.join(".synchub")).unwrap();
        let workspace = WorkspaceSnapshot {
            entry: WorkspaceRegistryEntry {
                root: root.display().to_string(),
                remote_path: "/workspace".into(),
                ..Default::default()
            },
            manifest: Some(Manifest {
                items: vec![ManifestEntry {
                    relative_path: "deleted.txt".into(),
                    path: "/workspace/deleted.txt".into(),
                    size: 3,
                    sha256: "old".into(),
                    remote_version: Some(6),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            assert!(
                request.starts_with("GET /api/v1/files/by-path?path=%2Fworkspace%2Fdeleted.txt")
            );
            write_json_response(&mut stream, 200, r#"{"code":0,"message":"ok","data":{"id":"file_1","name":"deleted.txt","path":"/workspace/deleted.txt","node_type":"file","size":3,"version":6}}"#).await;

            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            assert!(request.starts_with("DELETE /api/v1/files/file_1"));
            assert!(request.ends_with(r#"{"base_version":6}"#));
            write_json_response(&mut stream, 200, r#"{"code":0,"message":"ok","data":{}}"#).await;
        });

        let result = execute_push(
            &SyncHubClient::new(format!("http://{address}")).unwrap(),
            "token",
            &workspace,
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            PushResult {
                uploaded: 0,
                deleted: 1
            }
        );
        let manifest: Manifest =
            serde_json::from_str(&fs::read_to_string(root.join(".synchub/manifest.json")).unwrap())
                .unwrap();
        assert!(manifest.items.is_empty());
        server.await.unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "synchub-native-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn new_workspace(root: &Path) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            entry: WorkspaceRegistryEntry {
                root: root.display().to_string(),
                remote_path: "/workspace".into(),
                ..Default::default()
            },
            manifest: Some(Manifest::default()),
            ..Default::default()
        }
    }

    async fn upload_server(fail_upload: bool) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            let request_count = if fail_upload { 3 } else { 5 };
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                let line = request.lines().next().unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap();
                let path = parts.next().unwrap();
                requests.push(format!("{method} {path}"));
                let (status, body) = match (method, path) {
                    ("GET", _) => (404, r#"{"code":"NOT_FOUND","message":"not found"}"#),
                    ("POST", "/api/v1/files/directories") => (
                        201,
                        r#"{"code":0,"message":"ok","data":{"id":"dir_1","name":"workspace","path":"/workspace","node_type":"directory","size":0,"version":1}}"#,
                    ),
                    ("POST", "/api/v1/uploads") if fail_upload => {
                        (409, r#"{"code":"FILE_CONFLICT","message":"conflict"}"#)
                    }
                    ("POST", "/api/v1/uploads") => (
                        201,
                        r#"{"code":0,"message":"ok","data":{"upload_id":"upl_1","path":"/workspace/a.txt","chunk_size":8,"status":"pending"}}"#,
                    ),
                    ("PUT", "/api/v1/uploads/upl_1/chunks/0") => (
                        200,
                        r#"{"code":0,"message":"ok","data":{"chunk_index":0,"size":5,"sha256":"hash"}}"#,
                    ),
                    ("POST", "/api/v1/uploads/upl_1/commit") => (
                        200,
                        r#"{"code":0,"message":"ok","data":{"file_id":"file_1","version":7,"change_id":9}}"#,
                    ),
                    _ => panic!("unexpected request: {method} {path}"),
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            data.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&data);
            let Some(headers_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let content_length = text[..headers_end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if data.len() >= headers_end + 4 + content_length {
                return String::from_utf8(data).unwrap();
            }
        }
    }

    async fn write_json_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }
}

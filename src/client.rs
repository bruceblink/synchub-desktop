use crate::models::{
    ApiEnvelope, ApiStatus, ChangeListData, CliConfig, CommitUploadData, Device, DeviceListData,
    FileListData, FileNode, FileVersion, FileVersionListData, LoginData, RestoreFileVersionData,
    SyncConflict, SyncConflictListData, TokenPair, UploadChunk, UploadSession, VersionInfo,
    is_success_code,
};
use anyhow::{Context, Result, anyhow};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::time::{Duration, SystemTime};

#[derive(Clone)]
pub struct SyncHubClient {
    base_url: String,
    client: Client,
}

impl SyncHubClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(20)).build()?;
        Ok(Self {
            base_url: normalize_base_url(&base_url.into()),
            client,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn version(&self) -> Result<VersionInfo> {
        self.request_json(Method::GET, "/version", None, None).await
    }

    pub async fn health(&self) -> Result<ApiStatus> {
        self.request_json(Method::GET, "/healthz", None, None).await
    }

    pub async fn ready(&self) -> Result<ApiStatus> {
        self.request_json(Method::GET, "/readyz", None, None).await
    }

    pub async fn metrics(&self) -> Result<String> {
        self.request_text(Method::GET, "/metrics", None, None).await
    }

    pub async fn openapi(&self) -> Result<String> {
        self.request_text(Method::GET, "/swagger/openapi.yaml", None, None)
            .await
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<LoginData> {
        self.request_json(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": email, "password": password })),
        )
        .await
    }

    pub async fn register(&self, email: &str, password: &str) -> Result<LoginData> {
        self.request_json(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(json!({ "email": email, "password": password })),
        )
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenPair> {
        self.request_json(
            Method::POST,
            "/api/v1/auth/refresh",
            None,
            Some(json!({ "refresh_token": refresh_token })),
        )
        .await
    }

    pub async fn logout(&self, refresh_token: &str) -> Result<()> {
        self.request_empty(
            Method::POST,
            "/api/v1/auth/logout",
            None,
            Some(json!({ "refresh_token": refresh_token })),
        )
        .await
    }

    pub async fn get_file_by_path(&self, access_token: &str, path: &str) -> Result<FileNode> {
        let path = file_by_path_endpoint(path);
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn download_file(&self, access_token: &str, file_id: &str) -> Result<Vec<u8>> {
        let path = format!("/api/v1/files/{}/content", url_escape(file_id));
        let response = self
            .client
            .get(endpoint(&self.base_url, &path))
            .bearer_auth(access_token)
            .send()
            .await
            .context("download remote file")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.context("decode download error")?;
            if let Ok(envelope) = serde_json::from_str::<ApiEnvelope<serde_json::Value>>(&body)
                && !envelope.message.is_empty()
            {
                return Err(anyhow!(envelope.message));
            }
            return Err(anyhow!("download failed with status {}", status.as_u16()));
        }
        Ok(response
            .bytes()
            .await
            .context("read downloaded file")?
            .to_vec())
    }

    pub async fn list_files(
        &self,
        access_token: &str,
        page_size: u32,
        cursor: Option<&str>,
    ) -> Result<FileListData> {
        self.list_files_page(access_token, None, page_size, cursor)
            .await
    }

    pub async fn list_files_for_path(
        &self,
        access_token: &str,
        remote_path: &str,
        page_size: u32,
        cursor: Option<&str>,
    ) -> Result<FileListData> {
        let remote_path = remote_path.trim();
        if remote_path.is_empty() || remote_path == "/" {
            return self.list_files(access_token, page_size, cursor).await;
        }

        let parent = self.get_file_by_path(access_token, remote_path).await?;
        if parent.node_type != "directory" {
            return Err(anyhow!("remote workspace path is not a directory"));
        }
        self.list_files_page(access_token, Some(parent.id.as_str()), page_size, cursor)
            .await
    }

    async fn list_files_page(
        &self,
        access_token: &str,
        parent_id: Option<&str>,
        page_size: u32,
        cursor: Option<&str>,
    ) -> Result<FileListData> {
        let path = files_endpoint(parent_id, page_size, cursor);
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn create_directory(
        &self,
        access_token: &str,
        path: &str,
        device_id: Option<&str>,
    ) -> Result<FileNode> {
        self.request_json(
            Method::POST,
            "/api/v1/files/directories",
            Some(access_token),
            Some(path_device_body(path, device_id)),
        )
        .await
    }

    pub async fn move_file(
        &self,
        access_token: &str,
        file_id: &str,
        path: &str,
        device_id: Option<&str>,
    ) -> Result<FileNode> {
        let endpoint = format!("/api/v1/files/{}", file_id);
        self.request_json(
            Method::PATCH,
            &endpoint,
            Some(access_token),
            Some(path_device_body(path, device_id)),
        )
        .await
    }

    pub async fn delete_file(
        &self,
        access_token: &str,
        file_id: &str,
        device_id: Option<&str>,
    ) -> Result<()> {
        let path = format!("/api/v1/files/{}", file_id);
        self.request_empty(
            Method::DELETE,
            &path,
            Some(access_token),
            Some(device_body(device_id)),
        )
        .await
    }

    pub async fn delete_file_versioned(
        &self,
        access_token: &str,
        file_id: &str,
        device_id: Option<&str>,
        base_version: Option<i64>,
    ) -> Result<()> {
        let path = format!("/api/v1/files/{}", url_escape(file_id));
        let mut body = device_body(device_id);
        if let Some(version) = base_version {
            body["base_version"] = json!(version);
        }
        self.request_empty(Method::DELETE, &path, Some(access_token), Some(body))
            .await
    }

    pub async fn init_upload(
        &self,
        access_token: &str,
        path: &str,
        size: i64,
        sha256: &str,
        base_version: Option<i64>,
        device_id: Option<&str>,
        idempotency_key: &str,
    ) -> Result<UploadSession> {
        let mut body = json!({ "path": path, "size": size, "sha256": sha256 });
        if let Some(version) = base_version {
            body["base_version"] = json!(version);
        }
        if let Some(device_id) = device_id.filter(|value| !value.trim().is_empty()) {
            body["device_id"] = json!(device_id);
        }
        let url = endpoint(&self.base_url, "/api/v1/uploads");
        let response = self
            .client
            .post(url)
            .bearer_auth(access_token)
            .header("Idempotency-Key", idempotency_key)
            .json(&body)
            .send()
            .await
            .context("initialize upload")?;
        decode_json_response(response).await
    }

    pub async fn put_upload_chunk(
        &self,
        access_token: &str,
        upload_id: &str,
        index: i32,
        content: Vec<u8>,
        sha256: &str,
    ) -> Result<UploadChunk> {
        let path = format!("/api/v1/uploads/{}/chunks/{index}", url_escape(upload_id));
        let response = self
            .client
            .put(endpoint(&self.base_url, &path))
            .bearer_auth(access_token)
            .header("Content-Type", "application/octet-stream")
            .header("X-Chunk-Sha256", sha256)
            .body(content)
            .send()
            .await
            .context("upload file chunk")?;
        decode_json_response(response).await
    }

    pub async fn commit_upload(
        &self,
        access_token: &str,
        upload_id: &str,
    ) -> Result<CommitUploadData> {
        let path = format!("/api/v1/uploads/{}/commit", url_escape(upload_id));
        self.request_json(Method::POST, &path, Some(access_token), Some(json!({})))
            .await
    }

    pub async fn list_trash(&self, access_token: &str) -> Result<Vec<FileNode>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let path = trash_endpoint(200, cursor.as_deref());
            let page: FileListData = self
                .request_json(Method::GET, &path, Some(access_token), None)
                .await?;
            items.extend(page.items);
            cursor = page.next_cursor.filter(|value| !value.trim().is_empty());
            if cursor.is_none() {
                return Ok(items);
            }
        }
    }

    pub async fn restore_trash(
        &self,
        access_token: &str,
        file_id: &str,
        device_id: Option<&str>,
    ) -> Result<FileNode> {
        let path = format!("/api/v1/trash/{}/restore", url_escape(file_id));
        self.request_json(
            Method::POST,
            &path,
            Some(access_token),
            Some(device_body(device_id)),
        )
        .await
    }

    pub async fn list_file_versions(
        &self,
        access_token: &str,
        file_id: &str,
        limit: u32,
    ) -> Result<FileVersionListData> {
        let path = format!(
            "/api/v1/files/{}/versions?limit={}",
            url_escape(file_id),
            limit
        );
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn restore_file_version(
        &self,
        access_token: &str,
        file_id: &str,
        version: i64,
        device_id: Option<&str>,
    ) -> Result<RestoreFileVersionData> {
        let path = format!(
            "/api/v1/files/{}/versions/{version}/restore",
            url_escape(file_id)
        );
        self.request_json(
            Method::POST,
            &path,
            Some(access_token),
            Some(device_body(device_id)),
        )
        .await
    }

    pub async fn pin_file_version(
        &self,
        access_token: &str,
        file_id: &str,
        version: i64,
    ) -> Result<FileVersion> {
        let path = format!(
            "/api/v1/files/{}/versions/{version}/pin",
            url_escape(file_id)
        );
        self.request_json(Method::POST, &path, Some(access_token), Some(json!({})))
            .await
    }

    pub async fn unpin_file_version(
        &self,
        access_token: &str,
        file_id: &str,
        version: i64,
    ) -> Result<FileVersion> {
        let path = format!(
            "/api/v1/files/{}/versions/{version}/pin",
            url_escape(file_id)
        );
        self.request_json(Method::DELETE, &path, Some(access_token), None)
            .await
    }

    pub async fn list_conflicts(
        &self,
        access_token: &str,
        limit: u32,
    ) -> Result<SyncConflictListData> {
        let path = format!("/api/v1/sync/conflicts?resolution=pending&limit={}", limit);
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn list_devices(&self, access_token: &str, limit: u32) -> Result<DeviceListData> {
        let path = format!("/api/v1/devices?limit={}", limit);
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn register_device(
        &self,
        access_token: &str,
        name: &str,
        platform: &str,
    ) -> Result<Device> {
        self.request_json(
            Method::POST,
            "/api/v1/devices",
            Some(access_token),
            Some(json!({ "name": name, "platform": platform })),
        )
        .await
    }

    pub async fn report_device_sync(
        &self,
        access_token: &str,
        device_id: &str,
        status: &str,
        error: &str,
    ) -> Result<Device> {
        let path = format!("/api/v1/devices/{}/heartbeat", url_escape(device_id));
        self.request_json(
            Method::POST,
            &path,
            Some(access_token),
            Some(json!({ "status": status, "error": error })),
        )
        .await
    }

    pub async fn list_changes(
        &self,
        access_token: &str,
        device_id: &str,
        after_change_id: i64,
        limit: u32,
    ) -> Result<ChangeListData> {
        let path = format!(
            "/api/v1/sync/changes?device_id={}&after_change_id={after_change_id}&limit={}",
            url_escape(device_id),
            limit.clamp(1, 500)
        );
        self.request_json(Method::GET, &path, Some(access_token), None)
            .await
    }

    pub async fn ack_changes(
        &self,
        access_token: &str,
        device_id: &str,
        last_applied_change_id: i64,
    ) -> Result<Device> {
        self.request_json(
            Method::POST,
            "/api/v1/sync/ack",
            Some(access_token),
            Some(json!({
                "device_id": device_id,
                "last_applied_change_id": last_applied_change_id,
            })),
        )
        .await
    }

    pub async fn resolve_conflict(
        &self,
        access_token: &str,
        conflict_id: &str,
        resolution: &str,
    ) -> Result<SyncConflict> {
        let path = format!("/api/v1/sync/conflicts/{}", conflict_id);
        self.request_json(
            Method::PATCH,
            &path,
            Some(access_token),
            Some(json!({ "resolution": resolution })),
        )
        .await
    }

    async fn request_empty(
        &self,
        method: Method,
        path: &str,
        access_token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Result<()> {
        let _: serde_json::Value = self.request_json(method, path, access_token, body).await?;
        Ok(())
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        access_token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let url = endpoint(&self.base_url, path);
        let mut request = self.client.request(method, url);
        if let Some(token) = access_token.filter(|token| !token.trim().is_empty()) {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.context("send request")?;
        let status = response.status();
        let envelope: ApiEnvelope<T> = response.json().await.context("decode response")?;
        if !status.is_success() || !is_success_code(&envelope.code) {
            return Err(anyhow!(if envelope.message.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                envelope.message
            }));
        }
        envelope
            .data
            .ok_or_else(|| anyhow!("response data is empty"))
    }

    async fn request_text(
        &self,
        method: Method,
        path: &str,
        access_token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Result<String> {
        let url = endpoint(&self.base_url, path);
        let mut request = self.client.request(method, url);
        if let Some(token) = access_token.filter(|token| !token.trim().is_empty()) {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.context("send request")?;
        let status = response.status();
        let body = response.text().await.context("decode response")?;
        if !status.is_success() {
            if let Ok(envelope) = serde_json::from_str::<ApiEnvelope<serde_json::Value>>(&body) {
                if !envelope.message.is_empty() {
                    return Err(anyhow!(envelope.message));
                }
            }
            return Err(anyhow!("request failed with status {}", status.as_u16()));
        }
        Ok(body)
    }
}

async fn decode_json_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let envelope: ApiEnvelope<T> = response.json().await.context("decode response")?;
    if !status.is_success() || !is_success_code(&envelope.code) {
        return Err(anyhow!(if envelope.message.is_empty() {
            format!("request failed with status {}", status.as_u16())
        } else {
            envelope.message
        }));
    }
    envelope
        .data
        .ok_or_else(|| anyhow!("response data is empty"))
}

fn path_device_body(path: &str, device_id: Option<&str>) -> serde_json::Value {
    let mut body = json!({ "path": path });
    if let Some(device_id) = device_id.filter(|value| !value.trim().is_empty()) {
        body["device_id"] = json!(device_id);
    }
    body
}

fn device_body(device_id: Option<&str>) -> serde_json::Value {
    let mut body = json!({});
    if let Some(device_id) = device_id.filter(|value| !value.trim().is_empty()) {
        body["device_id"] = json!(device_id);
    }
    body
}

pub async fn refresh_cli_config_if_needed(config: &mut CliConfig) -> Result<bool> {
    if config.tokens.refresh_token.trim().is_empty() {
        return Ok(false);
    }
    let should_refresh = config
        .access_token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|expires| expires <= SystemTime::now() + Duration::from_secs(60))
        .unwrap_or(false);
    if !should_refresh {
        return Ok(false);
    }
    let client = SyncHubClient::new(&config.server_url)?;
    let tokens = client.refresh(&config.tokens.refresh_token).await?;
    config.tokens = tokens;
    Ok(true)
}

pub fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let with_scheme = if trimmed.is_empty() {
        "http://localhost:8765".to_string()
    } else if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    };
    with_scheme.trim_end_matches('/').to_string()
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        normalize_base_url(base_url),
        path.trim_start_matches('/')
    )
}

fn file_by_path_endpoint(path: &str) -> String {
    format!("/api/v1/files/by-path?path={}", url_escape(path.trim()))
}

fn files_endpoint(parent_id: Option<&str>, page_size: u32, cursor: Option<&str>) -> String {
    let mut query = Vec::new();
    if let Some(parent_id) = parent_id.filter(|value| !value.trim().is_empty()) {
        query.push(format!("parent_id={}", url_escape(parent_id.trim())));
    }
    if let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) {
        query.push(format!("cursor={}", url_escape(cursor.trim())));
    }
    if page_size > 0 {
        query.push(format!("page_size={page_size}"));
    }
    if query.is_empty() {
        "/api/v1/files".to_string()
    } else {
        format!("/api/v1/files?{}", query.join("&"))
    }
}

fn trash_endpoint(page_size: u32, cursor: Option<&str>) -> String {
    let mut path = format!("/api/v1/trash?page_size={}", page_size.clamp(1, 200));
    if let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) {
        path.push_str("&cursor=");
        path.push_str(&url_escape(cursor.trim()));
    }
    path
}

fn url_escape(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn parse_rfc3339_utc(value: &str) -> Option<SystemTime> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let trimmed = trimmed.strip_suffix('Z').unwrap_or(trimmed);
    let mut parts = trimmed.split('T');
    let date = parts.next()?;
    let time = parts.next()?;
    let mut date_parts = date.split('-').filter_map(|part| part.parse::<i64>().ok());
    let year = date_parts.next()?;
    let month = date_parts.next()?;
    let day = date_parts.next()?;
    let mut time_parts = time
        .split([':', '.'])
        .take(3)
        .filter_map(|part| part.parse::<i64>().ok());
    let hour = time_parts.next()?;
    let minute = time_parts.next()?;
    let second = time_parts.next()?;
    let days = days_from_civil(year, month, day)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    if seconds < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - (month <= 2) as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn normalizes_base_url() {
        assert_eq!(
            normalize_base_url("localhost:8765/"),
            "http://localhost:8765"
        );
        assert_eq!(
            normalize_base_url("https://sync.example/"),
            "https://sync.example"
        );
    }

    #[tokio::test]
    async fn downloads_binary_file_with_bearer_auth() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /api/v1/files/file%201/content HTTP/1.1"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer token-1")
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n\x00\x01\x02\x03")
                .await
                .unwrap();
        });

        let client = SyncHubClient::new(format!("http://{address}")).unwrap();
        assert_eq!(
            client.download_file("token-1", "file 1").await.unwrap(),
            [0, 1, 2, 3]
        );
        server.await.unwrap();
    }

    #[test]
    fn path_device_body_omits_empty_device_id() {
        assert_eq!(
            path_device_body("/workspace/docs", Some("dev_1")),
            json!({ "path": "/workspace/docs", "device_id": "dev_1" })
        );
        assert_eq!(
            path_device_body("/workspace/docs", Some("")),
            json!({ "path": "/workspace/docs" })
        );
    }

    #[test]
    fn device_body_omits_empty_device_id() {
        assert_eq!(device_body(Some("dev_1")), json!({ "device_id": "dev_1" }));
        assert_eq!(device_body(Some("")), json!({}));
    }

    #[test]
    fn url_escape_encodes_path_segments() {
        assert_eq!(url_escape("file 1/版本"), "file%201%2F%E7%89%88%E6%9C%AC");
        assert_eq!(url_escape("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn file_by_path_endpoint_escapes_remote_path_query() {
        assert_eq!(
            file_by_path_endpoint("/workspace/docs/readme.md"),
            "/api/v1/files/by-path?path=%2Fworkspace%2Fdocs%2Freadme.md"
        );
    }

    #[test]
    fn files_endpoint_scopes_list_to_parent_directory() {
        assert_eq!(
            files_endpoint(None, 100, None),
            "/api/v1/files?page_size=100"
        );
        assert_eq!(
            files_endpoint(Some("dir 1"), 25, Some("cursor/next")),
            "/api/v1/files?parent_id=dir%201&cursor=cursor%2Fnext&page_size=25"
        );
    }

    #[test]
    fn trash_endpoint_clamps_page_size_and_escapes_cursor() {
        assert_eq!(trash_endpoint(0, None), "/api/v1/trash?page_size=1");
        assert_eq!(
            trash_endpoint(500, Some("next/cursor")),
            "/api/v1/trash?page_size=200&cursor=next%2Fcursor"
        );
    }
}

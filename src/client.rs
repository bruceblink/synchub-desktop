use crate::models::{
    ApiEnvelope, ApiStatus, CliConfig, DeviceListData, FileListData, FileNode, LoginData,
    SyncConflict, SyncConflictListData, TokenPair, is_success_code,
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

    pub async fn ready(&self) -> Result<ApiStatus> {
        self.request_json(Method::GET, "/readyz", None, None).await
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

    pub async fn list_files(&self, access_token: &str, page_size: u32) -> Result<FileListData> {
        let path = format!("/api/v1/files?page_size={}", page_size);
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
            Some(create_directory_body(path, device_id)),
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
}

fn create_directory_body(path: &str, device_id: Option<&str>) -> serde_json::Value {
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

    #[test]
    fn create_directory_body_omits_empty_device_id() {
        assert_eq!(
            create_directory_body("/workspace/docs", Some("dev_1")),
            json!({ "path": "/workspace/docs", "device_id": "dev_1" })
        );
        assert_eq!(
            create_directory_body("/workspace/docs", Some("")),
            json!({ "path": "/workspace/docs" })
        );
    }

    #[test]
    fn device_body_omits_empty_device_id() {
        assert_eq!(device_body(Some("dev_1")), json!({ "device_id": "dev_1" }));
        assert_eq!(device_body(Some("")), json!({}));
    }
}

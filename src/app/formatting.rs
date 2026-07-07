use crate::models::Device;
pub(super) fn device_name(device: &Device) -> &str {
    if device.name.trim().is_empty() {
        "unnamed device"
    } else {
        device.name.as_str()
    }
}

pub(super) fn format_optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

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

pub(super) fn short_hash(value: &str) -> String {
    let value = value.trim();
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(12).collect::<String>();
    if chars.next().is_none() {
        value.to_string()
    } else {
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_shortened_for_dense_rows() {
        assert_eq!(short_hash("abcdef1234567890"), "abcdef123456...");
        assert_eq!(short_hash("abc"), "abc");
    }
}

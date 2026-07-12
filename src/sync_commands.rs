pub fn parse_workspace_paths(input: &str) -> Vec<String> {
    input
        .split(['\n', '\r', ';'])
        .map(|path| path.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn sync_action_label(action: &str) -> &'static str {
    match action {
        "status" => "Status",
        "doctor" => "Doctor",
        "dry-run" => "Dry Run",
        "once" => "Sync Once",
        "push" => "Push",
        "pull" => "Pull",
        _ => "Status",
    }
}

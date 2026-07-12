pub fn sync_command_args(action: &str, root: &str, config: &str) -> Option<Vec<String>> {
    let mut args = vec!["sync".to_string()];
    match action {
        "status" => args.extend([
            "status".to_string(),
            "--show-remote".to_string(),
            "--show-conflicts".to_string(),
        ]),
        "doctor" => args.push("doctor".to_string()),
        "dry-run" => args.extend(["once".to_string(), "--dry-run".to_string()]),
        "once" | "push" | "pull" => args.push(action.to_string()),
        _ => return None,
    }
    args.extend([
        "--path".to_string(),
        root.to_string(),
        "--config".to_string(),
        config.to_string(),
    ]);
    Some(args)
}

pub fn parse_workspace_paths(input: &str) -> Vec<String> {
    input
        .split(['\n', '\r', ';'])
        .map(|path| path.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn file_download_command_args(
    root: &str,
    workspace_config: &str,
    config: &str,
    file_id: &str,
) -> Option<Vec<String>> {
    let root = root.trim();
    let file_id = file_id.trim();
    if root.is_empty() || file_id.is_empty() {
        return None;
    }

    let mut args = vec![
        "file".to_string(),
        "download".to_string(),
        "--path".to_string(),
        root.to_string(),
    ];
    let workspace_config = workspace_config.trim();
    if !workspace_config.is_empty() {
        args.extend([
            "--workspace-config".to_string(),
            workspace_config.to_string(),
        ]);
    }
    let config = config.trim();
    if !config.is_empty() {
        args.extend(["--config".to_string(), config.to_string()]);
    }
    args.extend([
        "--file-id".to_string(),
        file_id.to_string(),
        "--json".to_string(),
    ]);
    Some(args)
}

pub fn daemon_command_args(action: &str, root: &str, config: &str) -> Option<Vec<String>> {
    let root = root.trim();
    if root.is_empty() {
        return None;
    }

    let mut args = vec!["sync".to_string(), "daemon".to_string()];
    match action {
        "start" => {}
        "status" => args.push("--status".to_string()),
        "pause" => args.push("--pause".to_string()),
        "resume" => args.push("--resume".to_string()),
        "reset-state" => args.push("--reset-state".to_string()),
        _ => return None,
    }
    args.extend(["--path".to_string(), root.to_string()]);

    let config = config.trim();
    if !config.is_empty() {
        args.extend(["--config".to_string(), config.to_string()]);
    }
    Some(args)
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

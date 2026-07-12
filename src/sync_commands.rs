pub fn sync_command_args(action: &str, root: &str, config: &str) -> Option<Vec<String>> {
    let mut args = vec!["sync".to_string()];
    match action {
        "doctor" => args.push("doctor".to_string()),
        "once" | "pull" => args.push(action.to_string()),
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

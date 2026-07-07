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

pub fn workspace_init_command_args(
    roots: &[String],
    remote_root: &str,
    config: &str,
) -> Option<Vec<String>> {
    let roots = roots
        .iter()
        .map(|root| root.trim())
        .filter(|root| !root.is_empty())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return None;
    }

    let mut args = vec!["workspace".to_string(), "init".to_string()];
    for root in roots {
        args.extend(["--path".to_string(), root.to_string()]);
    }
    let remote_root = remote_root.trim();
    if !remote_root.is_empty() {
        args.extend(["--remote-root".to_string(), remote_root.to_string()]);
    }
    args.extend(["--config".to_string(), config.to_string()]);
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

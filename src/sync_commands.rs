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

pub fn workspace_remove_command_args(root: &str, config: &str) -> Option<Vec<String>> {
    let root = root.trim();
    if root.is_empty() {
        return None;
    }

    let mut args = vec![
        "workspace".to_string(),
        "remove".to_string(),
        "--path".to_string(),
        root.to_string(),
    ];
    let config = config.trim();
    if !config.is_empty() {
        args.extend(["--config".to_string(), config.to_string()]);
    }
    args.push("--json".to_string());
    Some(args)
}

pub fn workspace_prune_command_args(config: &str) -> Vec<String> {
    let mut args = vec!["workspace".to_string(), "prune".to_string()];
    let config = config.trim();
    if !config.is_empty() {
        args.extend(["--config".to_string(), config.to_string()]);
    }
    args.push("--json".to_string());
    args
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

pub fn trash_list_command_args(
    root: &str,
    workspace_config: &str,
    limit: usize,
) -> Option<Vec<String>> {
    let root = root.trim();
    if root.is_empty() || limit == 0 {
        return None;
    }

    let mut args = vec![
        "sync".to_string(),
        "trash".to_string(),
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
    args.extend([
        "--limit".to_string(),
        limit.to_string(),
        "--json".to_string(),
    ]);
    Some(args)
}

pub fn trash_restore_command_args(
    root: &str,
    workspace_config: &str,
    batch: &str,
    entry: &str,
) -> Option<Vec<String>> {
    let root = root.trim();
    let batch = batch.trim();
    let entry = entry.trim().trim_matches('/').trim_matches('\\');
    if root.is_empty() || batch.is_empty() || entry.is_empty() {
        return None;
    }

    let mut args = vec![
        "sync".to_string(),
        "trash".to_string(),
        "restore".to_string(),
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
    args.extend([
        "--batch".to_string(),
        batch.to_string(),
        "--entry".to_string(),
        entry.to_string(),
    ]);
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

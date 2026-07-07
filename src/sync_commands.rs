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

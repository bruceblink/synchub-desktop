use super::CommandResult;
use crate::models::{FileNode, SyncTrashSnapshot, TrashEntry};
use crate::sync_commands::{
    file_download_command_args, sync_action_label, sync_command_args, trash_list_command_args,
    trash_restore_command_args, workspace_init_command_args, workspace_prune_command_args,
    workspace_remove_command_args,
};
use std::path::PathBuf;
use std::process::Command;
pub(super) fn run_synchub_cli_file_download(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
    config_path: &PathBuf,
    file: &FileNode,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let workspace_config = workspace_config.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = file_download_command_args(&root, &workspace_config, &config, &file.id) else {
        return CommandResult {
            ok: false,
            summary: "remote file id is required".to_string(),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    if result.ok {
        result.summary = format!("downloaded remote file {}", file.path);
    } else {
        result.summary = format!("download failed: {}", result.summary);
    }
    result
}

pub(super) fn run_synchub_cli_trash_list(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
) -> (CommandResult, Vec<TrashEntry>) {
    let root = workspace_root.display().to_string();
    let config = workspace_config.display().to_string();
    let Some(args) = trash_list_command_args(&root, &config, 200) else {
        return (
            CommandResult {
                ok: false,
                summary: "workspace path is required".to_string(),
                output: String::new(),
            },
            Vec::new(),
        );
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    let entries = if result.ok {
        match serde_json::from_str::<SyncTrashSnapshot>(&result.output) {
            Ok(snapshot) => snapshot.items,
            Err(error) => {
                result.ok = false;
                result.summary = format!("decode trash failed: {error}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if result.ok {
        result.summary = format!("loaded {} trash item(s)", entries.len());
    } else {
        result.summary = format!("load trash failed: {}", result.summary);
    }
    (result, entries)
}

pub(super) fn run_synchub_cli_trash_restore(
    workspace_root: &PathBuf,
    workspace_config: &PathBuf,
    entry: &TrashEntry,
) -> (CommandResult, Option<Vec<TrashEntry>>) {
    let root = workspace_root.display().to_string();
    let config = workspace_config.display().to_string();
    let Some(args) = trash_restore_command_args(&root, &config, &entry.batch, &entry.path) else {
        return (
            CommandResult {
                ok: false,
                summary: "trash batch and entry are required".to_string(),
                output: String::new(),
            },
            None,
        );
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    if !result.ok {
        result.summary = format!("restore trash failed: {}", result.summary);
        return (result, None);
    }

    let restore_output = result.output.clone();
    let (list_result, entries) = run_synchub_cli_trash_list(workspace_root, workspace_config);
    result.output = if list_result.output.trim().is_empty() {
        restore_output
    } else {
        format!("{restore_output}\n{}", list_result.output)
    };
    result.ok = list_result.ok;
    result.summary = if list_result.ok {
        format!("restored trash item {}", entry.path)
    } else {
        format!(
            "restored {}, but refresh failed: {}",
            entry.path, list_result.summary
        )
    };
    (result, Some(entries))
}

pub(super) fn run_synchub_cli_daemon(
    action: &str,
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let mut args = vec!["sync", "daemon"];
    match action {
        "status" => args.push("--status"),
        "pause" => args.push("--pause"),
        "resume" => args.push("--resume"),
        "start" => {}
        _ => {}
    }
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    args.extend(["--path", &root, "--config", &config]);
    run_command("synchub-cli", &args)
}

pub(super) fn run_synchub_cli_workspace_init(
    roots: &[String],
    remote_root: &str,
    config_path: &PathBuf,
) -> CommandResult {
    let config = config_path.display().to_string();
    let Some(args) = workspace_init_command_args(roots, remote_root, &config) else {
        return CommandResult {
            ok: false,
            summary: "workspace path is required".to_string(),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("initialized {} workspace(s)", roots.len())
    } else {
        format!("workspace init failed: {}", result.summary)
    };
    result
}

pub(super) fn run_synchub_cli_workspace_remove(
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = workspace_remove_command_args(&root, &config) else {
        return CommandResult {
            ok: false,
            summary: "workspace path is required".to_string(),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("removed workspace registration {}", root)
    } else {
        format!("workspace remove failed: {}", result.summary)
    };
    result
}

pub(super) fn run_synchub_cli_workspace_prune(config_path: &PathBuf) -> CommandResult {
    let config = config_path.display().to_string();
    let args = workspace_prune_command_args(&config);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        "pruned stale workspace registrations".to_string()
    } else {
        format!("workspace prune failed: {}", result.summary)
    };
    result
}

pub(super) fn run_synchub_cli_sync(
    action: &str,
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = sync_command_args(action, &root, &config) else {
        return CommandResult {
            ok: false,
            summary: format!("unknown sync action: {action}"),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("{} completed", sync_action_label(action))
    } else {
        format!("{} failed: {}", sync_action_label(action), result.summary)
    };
    result
}

fn run_command(program: &str, args: &[&str]) -> CommandResult {
    let output = Command::new(program).args(args).output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.trim().is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}{stderr}")
            };
            CommandResult {
                ok: output.status.success(),
                summary: if output.status.success() {
                    "command completed".to_string()
                } else {
                    format!("command exited with {}", output.status)
                },
                output: combined,
            }
        }
        Err(error) => CommandResult {
            ok: false,
            summary: format!("failed to start {program}: {error}"),
            output: String::new(),
        },
    }
}

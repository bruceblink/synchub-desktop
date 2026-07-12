use super::CommandResult;
use crate::sync_commands::daemon_command_args;
use std::path::PathBuf;
use std::process::Command;
pub(super) fn run_synchub_cli_daemon(
    action: &str,
    workspace_root: &PathBuf,
    config_path: &PathBuf,
) -> CommandResult {
    let root = workspace_root.display().to_string();
    let config = config_path.display().to_string();
    let Some(args) = daemon_command_args(action, &root, &config) else {
        return CommandResult {
            ok: false,
            summary: format!("unknown daemon action: {action}"),
            output: String::new(),
        };
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut result = run_command("synchub-cli", &arg_refs);
    result.summary = if result.ok {
        format!("{} completed", daemon_action_label(action))
    } else {
        format!("{} failed: {}", daemon_action_label(action), result.summary)
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

fn daemon_action_label(action: &str) -> &'static str {
    match action {
        "start" => "Daemon start",
        "status" => "Daemon status",
        "pause" => "Daemon pause",
        "resume" => "Daemon resume",
        "reset-state" => "Daemon reset",
        _ => "Daemon command",
    }
}

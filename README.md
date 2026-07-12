# SyncHub Desktop

SyncHub Desktop is the native GPUI sync client for SyncHub. It talks directly to the SyncHub HTTP API and owns the complete end-user sync workflow without a CLI runtime.

## Current MVP

- Login/register/logout against a SyncHub server.
- Show server version, health, readiness component checks, metrics, OpenAPI spec, and local login profile.
- Discover existing registered workspaces and manage the registry natively.
- Initialize one or more workspace folders from the sidebar, optionally under a shared remote root.
- Remove selected workspace registrations and prune stale registry entries.
- Show workspace manifest, pending local changes, trash, daemon state, and pending remote conflicts.
- Scan and persist workspace manifests natively, including `.synchubignore`, SHA-256 fingerprints, and remote-version continuity.
- List remote files for the selected workspace.
- Create remote folders in the selected workspace.
- Move or rename remote files and folders from the selected workspace.
- Delete remote files or folders from the selected workspace.
- Download remote files into the selected workspace.
- Inspect, restore, pin, and unpin remote file versions.
- List local trash entries for the selected workspace and restore them.
- List and restore cloud trash separately from local deletion-protection copies.
- List registered sync devices and highlight the current workspace device.
- Preview, diagnose, push, pull, or run a complete sync for the selected workspace.
- Automatically run background sync for registered workspaces, with pause, resume, status, and state reset controls.
- Preserve local edits as conflict copies and move remote deletions into recoverable local trash.

## Build

GPUI on Windows needs the native MSVC toolchain. Use Developer PowerShell for VS 2022:

```powershell
cargo run
```

The app stores authoritative desktop preferences under the platform config directory. Existing SyncHub config and workspace registry files are read for a lossless upgrade from older releases; no CLI executable is invoked or required.

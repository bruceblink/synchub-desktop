# SyncHub Desktop

SyncHub Desktop is a native GPUI client for SyncHub. It is a separate Rust project that talks to the SyncHub HTTP API and reads the same local CLI config/workspace files where useful.

## Current MVP

- Login/register/logout against a SyncHub server.
- Show API readiness and local login profile.
- Discover registered workspaces from the CLI registry.
- Initialize one or more workspace folders from the sidebar, optionally under a shared remote root.
- Show workspace manifest, pending local changes, trash, daemon state, and pending remote conflicts.
- List remote files for the selected workspace.
- Create remote folders in the selected workspace.
- Delete remote files or folders from the selected workspace.
- List registered sync devices and highlight the current workspace device.
- Run common sync commands for the selected workspace: status, doctor, dry run, sync once, push, and pull.
- Start, pause, resume, and inspect the SyncHub CLI daemon by shelling out to `synchub-cli`.

## Build

GPUI on Windows needs the native MSVC toolchain. Use Developer PowerShell for VS 2022:

```powershell
cargo run
```

The app stores its own desktop preferences under the platform config directory and reuses the SyncHub CLI files under the user `SyncHub` config directory by default.

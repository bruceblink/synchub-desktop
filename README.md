# SyncHub Desktop

[中文](#中文) | [English](#english)

## 中文

SyncHub Desktop 是 SyncHub 的原生 GPUI 同步客户端。它直接调用 SyncHub REST API，在桌面进程内承载完整用户同步流程，不依赖 CLI 可执行文件。

### 功能

- 登录、注册、退出和 token 自动刷新
- 配置服务端地址并管理多个本地工作区
- 原生 manifest 扫描、`.synchubignore`、SHA-256 和远端版本连续性
- 同步预览、诊断、push、pull 和完整 Sync Once
- 登录后为所有注册工作区自动启动后台同步
- 暂停、恢复、查看和重置后台同步状态
- 浏览、创建、移动、删除和下载远端文件
- 查看、恢复、固定和取消固定文件版本
- 分别管理本地保护性回收站与云端回收站
- 查看设备、同步冲突并选择保留本地、远端或两者
- 在远端覆盖或删除前保留本地 conflict 副本

### 构建

Windows 上的 GPUI 需要 Visual Studio 2022 MSVC 工具链。使用 Developer PowerShell：

```powershell
cargo run
```

发布构建：

```powershell
cargo build --release
```

### 使用

1. 输入 SyncHub 服务端地址并保存。
2. 使用邮箱和密码登录或注册。
3. 在侧边栏输入一个或多个工作区目录，可用换行或分号分隔。
4. 可选填写远端根目录，然后初始化工作区。
5. 应用会自动启动后台同步；Sync 页面可手动预览、诊断或立即同步。

桌面设置中的服务端地址是权威配置。应用会读取旧版 SyncHub 登录配置和工作区 registry 以支持无损升级，但不会调用旧 CLI。

### 验证

```powershell
cargo fmt -- --check
cargo test
cargo check
cargo build --release
```

服务端仓库默认位于 `F:\project\SyncHub`。本地联调时先启动 API，再把桌面端服务地址设置为 `http://localhost:8765`。

## English

SyncHub Desktop is the native GPUI synchronization client for SyncHub. It talks directly to the SyncHub REST API and owns the complete end-user sync workflow inside the desktop process, without depending on a CLI executable.

### Features

- Sign in, register, sign out, and automatically refresh tokens
- Configure the server URL and manage multiple local workspaces
- Native manifest scanning, `.synchubignore`, SHA-256, and remote-version continuity
- Sync preview, diagnostics, push, pull, and complete Sync Once
- Automatic background synchronization for every registered workspace after sign-in
- Pause, resume, inspect, and reset background sync state
- Browse, create, move, delete, and download remote files
- Inspect, restore, pin, and unpin file versions
- Manage local protection trash separately from cloud trash
- Inspect devices and resolve conflicts by keeping local, remote, or both versions
- Preserve local conflict copies before remote overwrite or deletion

### Build

GPUI on Windows requires the Visual Studio 2022 MSVC toolchain. Use Developer PowerShell:

```powershell
cargo run
```

Release build:

```powershell
cargo build --release
```

### Usage

1. Enter and save the SyncHub server URL.
2. Sign in or register with an email address and password.
3. Enter one or more workspace directories in the sidebar, separated by newlines or semicolons.
4. Optionally provide a shared remote root, then initialize the workspaces.
5. Background sync starts automatically. Use the Sync view to preview, diagnose, or trigger immediate synchronization.

The server URL stored in desktop settings is authoritative. The application reads legacy SyncHub login and workspace registry files for lossless upgrades, but never invokes the retired CLI.

### Verification

```powershell
cargo fmt -- --check
cargo test
cargo check
cargo build --release
```

The server repository is expected at `F:\project\SyncHub` in the local development setup. Start the API first and configure the desktop server URL as `http://localhost:8765` for local integration testing.

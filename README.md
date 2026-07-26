# Codex++ for Linux

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ 图标" width="152">
</p>

<p align="center">
  <a href="https://github.com/MisakiSATA/CodexPlusPlus/releases"><img alt="Release" src="https://img.shields.io/github/v/release/MisakiSATA/CodexPlusPlus"></a>
  <a href="https://github.com/MisakiSATA/CodexPlusPlus/actions/workflows/pr-build.yml"><img alt="Build" src="https://github.com/MisakiSATA/CodexPlusPlus/actions/workflows/pr-build.yml/badge.svg?branch=codex%2Flinux-full-stack"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Linux%20x86__64-FCC624?logo=linux&logoColor=black">
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/github/license/MisakiSATA/CodexPlusPlus"></a>
</p>

Codex++ for Linux 是 [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 的 Linux 专用 fork。它为 OpenAI Codex 桌面应用提供供应商切换、模型管理、会话工具和界面增强，并补齐了 Linux 启动、诊断、安装包与发布链路。

这个 fork 还解决了第三方模型上下文窗口被统一限制的问题：你可以为同一供应商中的不同模型分别声明 `1M`、`200K` 等窗口。Codex++ 会生成原生 `model_catalog_json`，让 Codex 按当前模型读取正确的上下文大小，同时保持干净的模型 ID。

> 本仓库当前只维护 Linux x86_64。Windows 与 macOS 请使用[上游项目](https://github.com/BigPizzaV3/CodexPlusPlus)。

> **开发状态：** Linux 全栈代码已在 `codex/linux-full-stack` 分支完成，但本 fork 目前还没有正式 GitHub Release。现阶段请从源码构建；下文的 `.deb`、`.rpm` 和便携包命令是首个 Release 发布后的安装路径。

## 与上游的主要差异

| 方向 | 本 fork 的实现 |
| --- | --- |
| Linux 桌面支持 | 原生启动 Codex 可执行文件，兼容 X11 与 Wayland/XWayland，提供显示后端诊断和 WebKitGTK 黑屏回退 |
| Linux 分发 | Release workflow 可产出便携 zip、Debian/Ubuntu `.deb` 和 `.rpm`，并提供仓库内 Arch `PKGBUILD` |
| 每模型上下文 | 模型名称与上下文窗口分栏配置，自动生成并注入 `model_catalog_json` |
| 旧配置迁移 | 自动把 `model[1M]` 旧格式拆成无后缀模型 ID 和独立窗口映射 |
| 更新边界 | 用户级安装采用版本目录与原子切换；系统包交给 apt、dnf、zypper 或 pacman 管理 |
| CI | 在 Linux 上执行前端检查、Rust 测试、Release 构建、安装冒烟测试和产物校验 |

## 功能概览

- **供应商与协议**：官方登录、官方登录混入 API、纯 API、聚合供应商，以及 Responses / Chat Completions 协议转换。
- **模型与上下文**：模型列表、模型测试、每模型上下文窗口、profile 级自动压缩阈值和 Codex 原生 catalog 生成。
- **会话管理**：扫描本地会话、批量删除、Markdown 导出、Token 用量历史、项目移动与 Provider metadata 同步。
- **Codex 增强**：插件市场、模型白名单、中文界面、粘贴修复、会话宽度与滚动恢复、Goals、Stepwise 和自定义脚本。
- **开发工作流**：Upstream worktree、线程 ID、Zed Remote 项目识别与打开。
- **诊断维护**：应用路径检测、端口检查、Watcher、日志、健康检查和 Linux 显示后端状态。

Codex++ 通过 Chromium DevTools Protocol 和本地辅助服务工作，不修改官方应用的 `app.asar`。所有界面增强都可以单独关闭；关闭总开关后，它仍可作为供应商和启动管理工具使用。

## 系统要求

- Linux x86_64。
- GTK 3、WebKitGTK 4.1 和 Ayatana AppIndicator 运行库。
- 一个合法获取、可运行的 Codex 桌面应用 Linux Electron 目录。Codex++ 不分发 OpenAI Codex 本体；首次启动时需要在管理工具中选择它。
- 使用 Wayland 时建议保留 XWayland。当前管理工具会在需要时通过 XWayland 运行，并默认启用 WebKitGTK 软件合成回退以规避部分显卡驱动的黑屏问题。

Codex 目录至少应包含名为 `ChatGPT`、`Codex` 或 `codex` 的桌面可执行文件及配套的 `resources/app.asar`，例如：

```text
codex-desktop/
  Codex
  resources/
    app.asar
```

可以在管理工具中选择可执行文件或它的父目录。仅安装 `codex` CLI 不满足要求。本项目不提供 Codex 桌面应用的下载、转换或重新分发；请使用 OpenAI 提供或你有权使用的 Linux 构建。

## 从源码运行

在首个正式 Release 发布前，这是当前可用的安装方式。需要 Rust stable、Node.js 22、npm、C/C++ 构建工具，以及 GTK/WebKitGTK 开发库。

Debian/Ubuntu 构建依赖：

```bash
sudo apt install build-essential pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev
```

Arch Linux 构建依赖：

```bash
sudo pacman -S --needed base-devel rust nodejs npm \
  webkit2gtk-4.1 gtk3 libayatana-appindicator openssl
```

构建并启动管理工具：

```bash
git clone https://github.com/MisakiSATA/CodexPlusPlus.git
cd CodexPlusPlus
git switch codex/linux-full-stack

cd apps/codex-plus-manager
npm ci
npm run vite:build
cd ../..
cargo build --release

./target/release/codex-plus-plus-manager
```

完成供应商和 Codex 应用路径配置后，可以运行：

```bash
./target/release/codex-plus-plus
```

## Release 安装

首个正式版本发布后，从本仓库的 [GitHub Releases](https://github.com/MisakiSATA/CodexPlusPlus/releases) 下载对应产物。每个产物会附带 SHA-256 摘要文件，安装前可以运行：

```bash
sha256sum -c <asset-name>.sha256
```

### Debian / Ubuntu

```bash
sudo apt install ./codex-plus-plus_*_amd64.deb
```

### Fedora

```bash
sudo dnf install ./codex-plus-plus-*.x86_64.rpm
```

当前 rpm 产物只在 Ubuntu CI 中完成构建与文件布局检查，尚未在 Fedora/openSUSE 容器内做安装测试。Fedora 是目标环境；openSUSE 用户可自行用 `zypper install` 验证依赖兼容性。

### Arch Linux

仓库中的 [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD) 会把已发布的 Linux 便携包重新打包为 pacman 软件包：

```bash
cp /path/to/CodexPlusPlus-<version>-linux-x64.zip packaging/arch/
cd packaging/arch
makepkg -si
```

构建前请让 `PKGBUILD` 中的 `pkgver` 与下载的 Release 版本一致。

### 用户级便携安装

便携包不需要 root 权限。解压后在包目录运行：

```bash
bash install.sh
```

程序会安装到 `~/.local/lib/codex-plus-plus/`，并在 `~/.local/share/applications/` 创建两个入口：

- **Codex++**：应用当前配置并启动 Codex。
- **Codex++ 管理工具**：管理应用路径、供应商、模型、会话、增强功能和诊断。

`.deb`、`.rpm` 和 Arch 包安装到 `/usr`，应通过系统包管理器升级；这类安装会禁用应用内自更新。

## 首次使用

1. 打开 **Codex++ 管理工具**。
2. 在安装维护区域选择 Codex 桌面应用的 Linux 可执行文件。
3. 新建供应商，填写协议、Base URL、API Key 和默认模型。
4. 在模型列表中配置模型名称与对应的上下文窗口。
5. 保存并切换到该供应商，然后从 **Codex++** 入口启动 Codex。
6. 如果启动或请求失败，先运行模型测试、Provider Doctor，并查看关于页中的诊断日志。

真实 API Key 只应保存在本机。提交 issue 时不要上传 `auth.json`、完整 `config.toml`、日志中的凭据或含密钥的截图。

## 每模型上下文窗口

供应商编辑器的模型列表分为两列，两侧严格按行对应。例如：

| 模型名称 | 上下文窗口 |
| --- | ---: |
| `deepseek/deepseek-v4-flash` | `1M` |
| `claude-sonnet-4` | `200K` |
| `gpt-5.4` | 留空，使用默认值 |

窗口支持 `K`、`M` 或纯数字，单位按十进制计算：

```text
1M       = 1000000
200K     = 200000
1000000  = 1000000
```

当 `~/.codex/config.toml` 没有用户手写的 `model_catalog_json` 时，保存并应用 profile 后，Codex++ 会：

1. 把窗口映射保存在 profile 的 `model_windows` 字段中。
2. 在 `~/.codex/model-catalogs/` 生成该 profile 的 catalog。
3. 在 `~/.codex/config.toml` 注入相对路径形式的 `model_catalog_json`。
4. 向 Codex 写入不带 `[1M]` 等后缀的模型 ID，避免污染模型历史记录。

如果配置中已经存在手写或外部 `model_catalog_json`，Codex++ 会保留它且不会覆盖。此时模型列表中的窗口不会写入 Codex++ catalog；请先确认你希望继续维护外部 catalog，或移除该指针后重新应用 profile。

旧版 `deepseek-v4-pro[1M]` 语法仍会在加载时自动迁移，但新配置应使用左右分栏。未填写单模型窗口时，会回退到供应商的 `model_context_window` 或 Codex 模型元数据中的默认值。

自动压缩阈值目前仍是 profile 级配置，对应 `model_auto_compact_token_limit`；catalog 中每个模型的 `auto_compact_token_limit` 保持 `null`，由 Codex 按自身规则处理。

## 供应商模式

| 模式 | 用途 | 认证方式 |
| --- | --- | --- |
| 官方登录 | 只使用 ChatGPT / Codex 官方账号 | 保留官方登录状态，清理自定义 provider 和 API Key |
| 官方登录 + API | 保留官方账号和插件入口，模型请求走兼容 API | API Key 写入 provider bearer token |
| 纯 API | 完全使用自定义 Base URL 与 Key | 独立保存 `config.toml` 与 API Key |
| 聚合供应商 | 在多个普通 API 供应商之间路由 | 支持故障转移、轮转和权重策略 |

Chat Completions 供应商可以通过本地代理转换为 Codex 使用的 Responses 协议。切换供应商时，Codex++ 会先保存当前配置，再写入目标配置。

## 数据位置

| 内容 | 默认位置 |
| --- | --- |
| Codex 配置 | `~/.codex/config.toml` |
| Codex 登录状态 | `~/.codex/auth.json` |
| 每模型 catalog | `~/.codex/model-catalogs/` |
| Codex 本地数据库 | `~/.codex/sqlite/*.db`，旧版回退到 `~/.codex/state_5.sqlite` |
| Codex++ 状态与日志 | `~/.codex-session-delete/` |
| Provider 同步备份 | `~/.codex/backups_state/provider-sync/` |
| 用户级程序文件 | `~/.local/lib/codex-plus-plus/` |

修改供应商配置或本地会话数据前，建议先备份 `~/.codex/`。

## 常见问题

### 找不到 Codex 应用

Codex++ 不包含 Codex 桌面应用。请确认目录中同时存在桌面可执行文件和 `resources/app.asar`，再在管理工具中选择 `ChatGPT`、`Codex`、`codex` 或它们的父目录。不要选择 Codex CLI、`codex-plus-plus` 或 `codex-plus-plus-manager`。

### 切换供应商后请求失败

在供应商详情中运行模型测试或 Provider Doctor，并检查协议、Base URL、Key 和测试模型是否匹配。纯 API 与官方登录混入 API 使用不同的认证位置，不要手工复制两种模式的 `auth.json`。

### Wayland 下窗口黑屏或托盘异常

管理工具默认在 Wayland 会话中优先使用 XWayland，并关闭 WebKitGTK 合成模式。请确认系统安装了 XWayland，然后在关于页查看“显示后端”诊断结果。需要自行调试时，可以在启动前显式设置 `GDK_BACKEND` 或 `WEBKIT_DISABLE_COMPOSITING_MODE` 覆盖默认值。

### 为什么系统包不能应用内更新

安装到 `/usr` 或 `/opt` 的文件归系统包管理器所有。Codex++ 会拒绝直接覆盖这些文件，请使用 apt、dnf、zypper 或 pacman 更新。用户级便携安装不受这个限制。

## 开发

前端使用 React 19、TypeScript、Vite 和 Tauri 2，后端使用 Rust 2024 edition。

```bash
# 前端测试、类型检查与构建
cd apps/codex-plus-manager
npm ci
npm test
npm run check
npm run vite:build

# Rust 格式、测试与构建
cd ../..
cargo fmt --all -- --check
cargo test --workspace
cargo build --release
```

主要目录：

```text
apps/
  codex-plus-launcher/          静默启动入口
  codex-plus-manager/           Tauri 管理工具与 React 前端
assets/inject/                  Codex 渲染端增强脚本
crates/
  codex-plus-core/              启动、配置、catalog、更新与安装核心
  codex-plus-data/              会话、导出与 Provider 数据
scripts/installer/linux/        Linux 便携包、deb、rpm 打包脚本
packaging/arch/                 Arch Linux PKGBUILD
docs/                           调研、设计、实施计划与完成报告
```

发布 Release 时，[`release-assets.yml`](.github/workflows/release-assets.yml) 会构建 Linux 便携包、`.deb`、`.rpm`、对应的 SHA-256 文件和 `latest.json`。当前没有 ARM64、AppImage、Flatpak 或 Snap 产物。

## 已知限制

- 仅构建和维护 Linux x86_64。
- 本 fork 尚未发布正式 Release；当前需要从源码构建。
- Codex++ 依赖 Codex 桌面应用的页面结构、CDP 和本地数据格式；Codex 更新后，部分增强功能可能需要跟随适配。
- 便携版的默认应用内更新源仍继承上游仓库。使用本 fork 的 Release 时，建议从本仓库手动下载新版本；系统包始终通过发行版包管理器更新。
- `model_catalog_json` 是 Codex 原生机制，但第三方供应商是否真正支持声明的窗口大小，仍取决于上游模型和中转服务。
- Arch `PKGBUILD` 目前随仓库维护，尚未发布到 AUR。

## 上游与许可证

本项目基于 [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 开发。供应商管理、会话工具、注入增强和大量基础设施来自上游；本 fork 重点维护每模型上下文与 Linux 全栈支持。第三方代码与素材声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

CodexPlusPlus 采用 [GNU Affero General Public License v3.0](LICENSE)，SPDX 标识为 `AGPL-3.0-only`。OpenAI、ChatGPT 和 Codex 是其各自权利人的商标；本项目不是 OpenAI 官方产品，也不包含或授权分发 Codex 桌面应用本体。

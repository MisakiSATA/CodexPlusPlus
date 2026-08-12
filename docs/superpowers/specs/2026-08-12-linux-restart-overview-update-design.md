# Linux 重启链路与概览更新源调整设计

日期：2026-08-12

## 背景与证据

当前 Codex++ 正在正常运行，后端 `1.2.42` 可通过 `127.0.0.1:57321/backend/status`
返回 `status: ok`。因此本次排查和开发不能通过真实重启来复现问题，必须先保护当前
会话。

重启失败日志和进程状态显示：

- 管理器发出 `manager.launch_requested` 后，新的 launcher 立即记录
  `launcher.already_running`，守护端口为 `57320`；
- Linux 版本的 `stop_launcher_processes_and_wait()` 当前为空实现，旧 launcher
  未被终止；
- Codex 主进程随后退出，`9229` 暂时拒绝连接，新的 launcher 无法接管；
- 当前运行实例仍由一个 launcher、一个 `/usr/lib/chatgpt/ChatGPT` 主进程和
  helper 组成，不能在开发期间主动杀掉。

概览页还包含一个与 Codex++ 核心功能无关的 JOJO Code“官方中转站”推广卡片。关于页、
更新模块仍引用上游 `BigPizzaV3/CodexPlusPlus`，而用户仓库是
`MisakiSATA/CodexPlusPlus`。

## 目标

1. 修复 Linux 上 Codex++ 重启时旧 launcher 未退出导致的新 launcher 接管失败。
2. 删除概览页的 JOJO Code“官方中转站”推送/推广模块，不改变中转站配置页面。
3. 将关于页项目链接和实际 GitHub Release 更新源切换到
   `MisakiSATA/CodexPlusPlus`。
4. 保留主题市场、脚本市场、广告列表等独立内容源的现有上游仓库。
5. 全程不修改 Codex 会话、认证文件、用户配置凭据或系统包管理文件。

## 非目标

- 不在本次工作中执行真实 Codex 重启；部署后由用户在方便时手动验证。
- 不新增 launcher IPC 协议或改变 helper API。
- 不修改 `/usr/lib/chatgpt`、`/usr/lib/openai-codex-desktop` 或其他包管理器拥有的文件。
- 不把所有第三方内容市场统一迁移到用户仓库。

## 设计

### 1. Linux launcher 重启控制

在 `crates/codex-plus-core/src/watcher.rs` 增加 Linux launcher 进程扫描：

- 读取 `/proc/<pid>/exe` 和 `/proc/<pid>/cmdline`；
- 只匹配可执行文件名严格为 `codex-plus-plus` 的 launcher；
- 排除当前进程及其父进程链，避免管理器在扫描时误杀自身或受保护的祖先进程；
- 不匹配 `codex-plus-plus-manager`、Codex CLI、ChatGPT 子进程或其他包装脚本。

Linux 的 `stop_launcher_processes_and_wait()` 使用现有终止等待框架：

1. 快照待终止的旧 launcher PID；
2. 向这些 PID 发送 `SIGTERM`；
3. 等待 PID 不再运行，并额外等待守护端口 `57320` 不再监听；
4. 最长等待 `RESTART_STOP_WAIT_TIMEOUT_MS`（5 秒），超时写入已有诊断日志并继续，
   由新 launcher 的 guard 负责报告最终冲突。

端口等待是必要的：非托管的 Rust 子进程可能短暂变成 zombie，单纯等待 `/proc` 消失
不能证明其监听 socket 已释放；新 launcher 只应在 guard 端口释放后启动。

Codex 主进程的 Linux 识别保持当前布局兼容，但收紧新版路径判断：

- `/usr/lib/chatgpt/ChatGPT` 必须出现在 `argv[0]`；
- 含 `--type=...` 的 Chromium renderer、GPU、utility 子进程一律排除；
- 旧 `/usr/lib/openai-codex-desktop/.../resources/app.asar` 识别继续保留；
- 仅把 `/usr/lib/chatgpt/ChatGPT` 作为参数传给调试器、脚本或其他进程时不应被误认。

管理器的 `restart_codex_plus` 调用顺序保持不变：先停止 launcher，再停止 Codex，最后
异步启动新的静默 launcher。这样不会改变 Windows/macOS 行为，也不会让当前管理器窗口
参与被终止集合。

### 2. 概览页移除中转站推送模块

在 `apps/codex-plus-manager/src/App.tsx` 的 `OverviewScreen` 中删除整个
`jojocode-overview` Panel，包括标题、模型标签和“打开 JOJO Code”按钮。

同步删除：

- 仅由该卡片使用的 `Network` 图标导入（若没有其他引用）；
- `styles.css` 中 `jojocode-overview*` 专属规则及其响应式规则；
- 中英文翻译中仅用于该卡片的 JOJO Code 文案。

中转站 profile、官方模式切换、环境检测和 relay 页面全部保留。删除后概览页第一块
内容直接从健康检查开始，既不显示推送，也不改变配置数据。

### 3. 更新源迁移到用户仓库

在 `crates/codex-plus-core/src/update.rs`：

- `DEFAULT_REPOSITORY` 改为 `MisakiSATA/CodexPlusPlus`；
- `DEFAULT_LATEST_JSON_URL` 改为
  `https://github.com/MisakiSATA/CodexPlusPlus/releases/latest/download/latest.json`；
- 所有 `check_for_update` 调用继续使用该常量，避免在 UI 层重复拼接 URL。

在 `apps/codex-plus-manager/src/App.tsx` 的 `AboutScreen`：

- “项目地址”显示 `github.com/MisakiSATA/CodexPlusPlus`；
- “打开项目主页”和“反馈问题”均指向用户仓库；
- Discord、Telegram 等社区链接不在本次范围内。

测试 fixture 中的 GitHub release URL 和断言同步为用户仓库，测试仍使用静态 payload，
不访问网络。

## 错误处理与安全

- launcher 扫描失败按“没有可终止 launcher”处理，并记录诊断事件，不阻塞管理器启动；
- `SIGTERM` 失败或等待超时不会升级为 `SIGKILL`，避免误杀用户当前 Codex；
- 新 launcher 启动前 guard 端口仍被占用时，返回已有 `launcher.already_running` 错误，
  不重复启动第三个实例；
- 更新源只改变 HTTP 读取地址，不读取或打印任何认证凭据；
- 所有修改都在用户级仓库和用户级安装目录内完成，不写系统包文件。

## 测试与验收

### 重启链路

- 先添加失败测试：Linux launcher 过滤只返回精确 `codex-plus-plus`，保护当前进程
  祖先，并在 launcher 退出后等待 guard 端口释放；
- 添加负例：`gdb /usr/lib/chatgpt/ChatGPT`、renderer 和 `codex-plus-plus-manager`
  都不会被终止；
- 运行定向 watcher/launcher 测试，再运行完整
  `cargo test -p codex-plus-core`。

### 概览与更新源

- 前端源码测试断言 `OverviewScreen` 不再包含 JOJO Code 推送文案或
  `jojocode-overview`；
- 更新模块测试断言默认仓库和 latest JSON 地址为 `MisakiSATA/CodexPlusPlus`；
- 运行 manager 的现有测试和 TypeScript 检查（沿用仓库已有命令）。

### 部署后手动验收

用户方便时再点击“重启 Codex++”，确认：

1. 旧 launcher 退出且 `57320` 释放；
2. 新 launcher 启动 `/usr/lib/chatgpt/ChatGPT`；
3. `9229`、`57321` 恢复监听，概览显示健康；
4. 对话、配置和当前 Codex 工作内容不受影响；
5. 关于页只显示用户仓库，概览不再出现中转站推送卡片。

## 影响文件

- `crates/codex-plus-core/src/watcher.rs`
- `crates/codex-plus-core/tests/watcher.rs`
- `crates/codex-plus-core/src/update.rs`
- `crates/codex-plus-core/tests/updater.rs`
- `apps/codex-plus-manager/src/App.tsx`
- `apps/codex-plus-manager/src/styles.css`
- `apps/codex-plus-manager/src/i18n-en.ts`
- 可能的 manager 源码测试文件（仅增加行为断言）


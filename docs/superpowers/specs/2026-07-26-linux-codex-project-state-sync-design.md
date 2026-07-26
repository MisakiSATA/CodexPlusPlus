# Linux Codex 项目状态同步修复设计

日期：2026-07-26

## 背景与根因

Codex++ 管理器会在供应商同步、供应商切换和 Codex 启动前后保存并合并 `~/.codex/.codex-global-state.json`。当前 `codex_app_state` 同步逻辑无条件把 `/` 转换为 Windows 反斜杠，导致 Linux 项目路径从 `/home/...` 或 `/data/...` 变成 `\home\...`、`\data\...`。Codex 无法把这些字符串解析为 Linux 绝对路径，因此项目列表与管理器操作后的状态不一致，旧项目也无法再次打开。

现场日志确认 `launcher.after_provider_sync`、`manager.sync_providers_now.after` 和 `relay_switch.after` 都写入过 `electron-saved-workspace-roots`、`project-order` 或 `active-workspace-roots`；当前状态文件中也存在已损坏的反斜杠 Linux 路径。

## 目标

- 以 Codex 当前的 `.codex-global-state.json` 为本地项目列表的唯一数据源。
- 管理器只保护和展示 Codex 项目状态，不维护会反向覆盖 Codex 的第二套本地项目目录。
- 保留历史项目列表、排序、标签和线程工作区提示。
- 不从历史快照恢复活动项目；启动后由用户从保留的列表中手动选择。
- 自动识别并修复旧版本在 Linux 上写入的反斜杠绝对路径。
- 写入用户状态前创建可恢复备份。
- 不改变 Windows 路径及 UNC 路径的现有行为。

## 非目标

- 不重写会话数据库或修改会话所属项目。
- 不把管理器的会话 `cwd` 列表合并成 Codex 项目列表。
- 不新增独立项目数据库。
- 不修改远程 Zed 项目功能。
- 不自动打开任一历史项目。

## 方案比较

### 方案 A：禁用 Linux 状态同步

改动最少，但供应商同步或未来状态迁移仍可能丢失项目元数据，且不能自动修复已经损坏的路径。

### 方案 B：平台感知的 Codex 状态同步（采用）

保留现有备份与快照机制，把路径规范化改为平台感知，并明确区分持久项目目录和瞬时活动项目。改动集中、可回归测试，也能修复当前用户状态。

### 方案 C：重写统一项目目录服务

边界最完整，但需要新增 API、前端状态和迁移流程，超出本次故障范围。

## 设计

### 单一数据源

本地项目目录只读取 Codex 的 `.codex-global-state.json`：

- `electron-saved-workspace-roots`：保存的项目列表。
- `project-order`：项目显示顺序。
- `electron-workspace-root-labels`：项目标签。
- `thread-workspace-root-hints`、`thread-projectless-output-directories`、`thread-writable-roots`：线程关联信息。

管理器在供应商切换或启动同步前只捕获上述 Codex 状态的安全快照；同步后以当前 Codex 状态优先，快照仅补回被操作意外移除的安全字段。管理器自身的会话列表和远程项目列表不得写入这些字段。

`active-workspace-roots` 是瞬时选择，不进入可恢复项目快照，也不从旧快照合并回当前状态。这样保留项目目录而不自动重新打开历史项目。

### 跨平台路径规范化

路径规范化必须显式区分平台：

- Windows：延续盘符、反斜杠和 UNC 处理；比较时大小写不敏感。
- Linux/macOS：保留 `/` 分隔符；去除根目录以外的多余末尾 `/`；比较时大小写敏感。
- Linux 兼容修复：形如 `\home\...`、`\data\...` 且明显由旧同步器生成的绝对路径，转换回 `/home/...`、`/data/...`。
- 空白或空路径继续忽略。

规范化逻辑使用可注入的平台参数编写纯函数测试，运行于任一开发平台时都能覆盖 Windows 与 Unix 行为。

### 当前状态恢复

首次运行修复后的同步逻辑时：

1. 读取当前 Codex 状态。
2. 识别保存列表、排序、标签键以及线程提示中的旧 Linux 反斜杠路径。
3. 在 `~/.codex/backups_state/app-state-sync/<timestamp>/` 保存原始状态和元数据。
4. 原位写回修复后的 Unix 路径。
5. 保留保存项目列表，且不从快照恢复 `active-workspace-roots`。
6. 使用原子写入同步 `.codex-global-state.json` 和其 `.bak` 文件。

只修复 Codex 当前状态及 Codex++ 已有安全快照中的项目记录，不导入管理器或会话数据库中的额外目录。

### 错误处理与诊断

- 状态文件不存在时不创建空项目列表。
- JSON 无法解析时不覆盖原文件，并记录非敏感诊断事件。
- 修复和同步失败不阻止 Codex 启动，但日志必须包含阶段、错误和备份位置。
- 日志不得记录认证信息或完整状态内容。

## 测试

先新增失败测试，再实现最小修复：

- Unix 项目路径保持 `/home/...` 和 `/data/...`。
- 旧错误路径 `\home\...`、`\data\...` 修复为 Unix 绝对路径。
- Unix 路径去重区分大小写，Windows 路径去重不区分大小写。
- Windows 盘符和 UNC 路径保持既有结果。
- 保存项目、排序、标签和线程提示在同步后保留。
- 历史快照中的 `active-workspace-roots` 不恢复。
- 状态修改前生成备份，解析失败时原文件不变。
- 运行 `codex-plus-core`、`codex-plus-data` 相关测试以及整个工作区回归测试。

## 验收标准

- Linux 状态文件中不再因 Codex++ 操作出现反斜杠本地绝对路径。
- Codex++ 管理器执行供应商同步或切换后，Codex 项目列表保持可用。
- 通过 Codex++ 启动后，旧项目仍在列表中且可手动选择。
- 启动流程不会从安全快照自动恢复活动项目。
- 当前损坏状态有备份并恢复为有效 Linux 路径。
- 相关自动化测试及工作区回归测试通过。

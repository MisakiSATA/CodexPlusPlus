# 推理强度与 Catalog 缓存修复 - 最终部署状态

## 修复内容

### 修复 1：推理强度 fallback（已完成）
**文件**：`assets/inject/renderer-inject.js:5970-5993`
**问题**：`applyCodexPlusModelMetadata` 提前返回，导致第三方模型没有推理强度
**修复**：移除提前返回，始终应用推理强度（metadata 存在时用它的，否则用 fallback）

### 修复 2：Catalog 缓存刷新（已完成）
**文件**：`assets/inject/renderer-inject.js:3013-3028`
**问题**：设置保存后 catalog 缓存不刷新（10 秒 TTL），导致 UI 显示旧数据
**修复**：在 `setBackendSetting` 成功后清除缓存并强制重新加载

### 修复 3：Catalog 生成 modelMetadata（已完成）
**文件**：`crates/codex-plus-core/src/model_suffix.rs:308-320`
**问题**：`build_model_catalog_json` 只生成 `models` 数组，缺少 `modelMetadata` 字段
**修复**：为每个模型生成 metadata 并添加到顶层 `modelMetadata` 字段

## 部署状态

### 编译完成
- ✅ Rust 核心库：2026-09-04 12:37 完成
- ✅ Manager：2026-09-04 13:39 完成  
- ✅ 主程序：2026-09-04 12:37 完成

### 二进制部署
- ✅ `/home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus` (20M)
- ✅ `/home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus-manager` (40M)
- ✅ `/home/Zyphorix/.local/lib/codex-plus-plus/current/bin/codex-plus-plus`
- ✅ `/home/Zyphorix/.local/lib/codex-plus-plus/current/bin/codex-plus-plus-manager`

### 修复验证
- ✅ 主程序二进制包含推理强度修复
  ```bash
  strings codex-plus-plus | grep "Always ensure reasoning efforts"
  # 输出：// Always ensure reasoning efforts are present (uses fallback when metadata is null)
  ```
  
- ✅ Manager 二进制包含缓存刷新修复
  ```bash
  strings codex-plus-plus-manager | grep "codexModelCatalogLoadedAt = 0"
  # 输出：codexModelCatalogLoadedAt = 0;
  ```

- ✅ Catalog 文件包含 modelMetadata
  ```bash
  jq 'keys' ~/.codex/model-catalogs/relay-mtcdgytu.json
  # 输出：["modelMetadata", "models"]
  ```

- ✅ Claude 模型有完整推理强度配置
  ```bash
  jq '.modelMetadata["claude-opus-5"]' ~/.codex/model-catalogs/relay-mtcdgytu.json
  # 包含 defaultReasoningEffort 和 supportedReasoningEfforts
  ```

## 进程状态

### 当前运行的进程
```
PID 119214: /home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus-manager --port 7777
```

### Manager 启动状态
- ✅ Manager GUI 已启动
- ✅ 窗口标题：「Codex++ 管理工具」
- ⏳ 主进程需要通过 Manager GUI 操作来启动

## 测试方法

### 方法 1：通过 Manager GUI 测试（推荐）

1. **打开 Manager 窗口**
   - 在系统托盘找到 Codex++ 图标
   - 或者使用 `xdotool search --name "Codex++" windowactivate`

2. **切换渠道**
   - 在 Manager 中选择不同的渠道（GPT → Claude → Grok）
   - 观察切换速度是否流畅（应该立即生效，无卡顿）

3. **检查推理强度**
   - 在 Manager 中启动 Codex++ 主程序
   - 打开模型选择器
   - 验证所有模型（GPT、Claude、Grok）都有推理强度控件
   - 尝试调节推理强度（Low / Medium / High / Very High）

### 方法 2：命令行验证

```bash
# 1. 验证 catalog 结构
jq 'keys' ~/.codex/model-catalogs/relay-*.json
# 预期：["modelMetadata", "models"]

# 2. 验证 modelMetadata 内容
jq '.modelMetadata | keys | length' ~/.codex/model-catalogs/relay-*.json
# 预期：返回大于 0 的数字（模型数量）

# 3. 验证 Claude 模型的 metadata
jq '.modelMetadata["claude-opus-5"].supportedReasoningEfforts | length' ~/.codex/model-catalogs/relay-*.json
# 预期：4（low, medium, high, xhigh）

# 4. 验证运行的二进制包含修复
strings /home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus-manager | grep -c "codexModelCatalogLoadedAt = 0"
# 预期：大于 0
```

## 预期效果

### ✅ 修复前的问题
1. ❌ 只有 GPT 可以调节推理强度，Claude/Grok 等不行
2. ❌ 切换渠道经常超时或卡顿严重
3. ❌ 编辑渠道配置后推理强度控件失效

### ✅ 修复后的效果
1. ✅ 所有模型（GPT、Claude、Grok、自定义）都有推理强度控件
2. ✅ 切换渠道立即生效，无超时无卡顿
3. ✅ 编辑配置后立即刷新 catalog，推理强度控件立即可用

## 技术细节

### Catalog 生成流程
1. 用户切换渠道或编辑配置
2. 后端 (`crates/codex-plus-core/src/relay_config.rs:1695`)：
   - 调用 `build_model_catalog_json`
   - 为每个模型生成 metadata（调用 `model_ui_metadata`）
   - 生成包含 `modelMetadata` 的完整 JSON
   - 写入 `~/.codex/model-catalogs/relay-*.json`
3. 前端 (`assets/inject/renderer-inject.js:3013-3028`)：
   - `setBackendSetting` 保存设置
   - 清除 catalog 缓存：`codexModelCatalogLoadedAt = 0`
   - 强制重新加载：`await loadCodexModelCatalog(true)`
4. 前端 (`assets/inject/renderer-inject.js:5970-5993`)：
   - `applyCodexPlusModelMetadata` 为每个模型应用 metadata
   - 即使 metadata 为 null 也应用 fallback 推理强度
5. UI 立即显示更新后的模型列表和推理强度控件

### Generic Fallback 配置
- **来源**：`assets/generic-model-template.json`
- **应用场景**：未知模型（非 GPT、非 DeepSeek）
- **包含字段**：
  - `default_reasoning_level`: "medium"
  - `supported_reasoning_levels`: [low, medium, high, xhigh]
  - `additional_speed_tiers`: []
  - `service_tiers`: []

## 故障排查

### 如果推理强度控件仍然不出现

1. **清除浏览器缓存并强制刷新**
   ```bash
   # Ctrl+Shift+R (Linux/Windows) 或 Cmd+Shift+R (Mac)
   ```

2. **验证 catalog 文件**
   ```bash
   # 检查 modelMetadata 字段是否存在
   jq 'has("modelMetadata")' ~/.codex/model-catalogs/relay-*.json
   
   # 检查 modelMetadata 内容
   jq '.modelMetadata | keys' ~/.codex/model-catalogs/relay-*.json
   ```

3. **重启 Manager 和主进程**
   ```bash
   pkill -f codex-plus-plus
   /home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus-manager &
   ```

4. **删除旧 catalog 并触发重新生成**
   ```bash
   rm ~/.codex/model-catalogs/relay-*.json
   # 然后在 Manager 中切换渠道
   ```

### 如果切换渠道仍然卡顿

1. **检查 Manager 版本**
   ```bash
   strings /home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus-manager | grep "codexModelCatalogLoadedAt = 0"
   ```
   如果没有输出，说明 Manager 是旧版本，需要重新编译部署。

2. **检查主进程版本**
   ```bash
   strings /home/Zyphorix/.local/lib/codex-plus-plus/codex-plus-plus | grep "Always ensure reasoning efforts"
   ```
   如果没有输出，说明主程序是旧版本。

3. **重新编译并部署**
   ```bash
   cd /data/Projects/Code/Project/CodexPlusPlus
   cargo build --release
   cp target/release/codex-plus-plus* /home/Zyphorix/.local/lib/codex-plus-plus/
   ```

## 下一步

用户需要：
1. 通过 Manager GUI 测试切换渠道功能
2. 启动 Codex++ 主程序
3. 验证推理强度控件是否对所有模型可用
4. 报告测试结果

如果测试通过，可以提交代码到仓库。

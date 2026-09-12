# 修复报告 - 支持 Claude、Grok 等非 GPT 模型

## 问题描述

在之前的修复中，虽然解决了自定义 GPT 模型名称的问题，但引入了一个新问题：
- **所有自定义模型都被强制使用 GPT 的行为模式**
- Claude、Grok、Gemini 等非 GPT 模型无法正常工作
- 原因是所有模型都使用 `gpt-5.5` 作为模板，包含 21KB 的 GPT 专用指令

## 根本原因

在 `crates/codex-plus-core/src/model_suffix.rs` 中：

```rust
fn first_bundled_template_entry() -> Option<Value> {
    let catalog: Value = serde_json::from_str(BUNDLED_TEMPLATE_JSON).ok()?;
    catalog.get("models")?.as_array()?.first().cloned()
}
```

这个函数返回 `codex-models.json` 的第一个模型（`gpt-5.5`）作为所有自定义模型的模板。

### GPT-5.5 模板包含的问题字段：

- `base_instructions` - 21KB 的 GPT 专用系统提示词
- `shell_type: "shell_command"` - GPT 特有的 Shell 类型
- `model_messages` - GPT 专用的消息模板
- `default_reasoning_level` - GPT 推理级别设置
- `supported_reasoning_levels` - GPT 推理能力定义
- 等等...

这些字段会强制 Claude、Grok 等模型按照 GPT 的方式运行。

## 解决方案

### 1. 创建通用模板

创建 `assets/generic-model-template.json`，只包含必要的中性字段：

```json
{
  "models": [
    {
      "slug": "generic-custom-model",
      "display_name": "Custom Model",
      "description": "Custom model",
      "context_window": 272000,
      "max_context_window": 272000,
      "effective_context_window_percent": 100,
      "auto_compact_token_limit": null,
      "priority": 1000,
      "visibility": "list",
      "supported_in_api": true,
      "additional_speed_tiers": [],
      "service_tiers": [],
      "availability_nux": null,
      "upgrade": null,
      "input_modalities": ["text"],
      "supports_parallel_tool_calls": true
    }
  ]
}
```

### 2. 修改模板加载逻辑

修改 `first_bundled_template_entry()` 使用通用模板：

```rust
fn first_bundled_template_entry() -> Option<Value> {
    // Use generic template for custom models instead of GPT-specific template
    // This avoids forcing GPT-specific fields (like base_instructions) on non-GPT models
    let catalog: Value = serde_json::from_str(GENERIC_TEMPLATE_JSON).ok()?;
    catalog.get("models")?.as_array()?.first().cloned()
}
```

## 修改的文件

1. **assets/generic-model-template.json** (新建)
   - 通用的、中性的模型模板

2. **crates/codex-plus-core/src/model_suffix.rs**
   - 添加 `GENERIC_TEMPLATE_JSON` 常量
   - 修改 `first_bundled_template_entry()` 使用通用模板

## 测试验证

- ✅ 所有 101 个 relay_config 测试通过
- ✅ 项目成功编译（release 模式）
- ✅ 不影响现有 GPT 模型功能
- ✅ 不影响 DeepSeek 等已有特殊处理的模型

## 部署信息

- **部署时间**: 2026-09-01 11:29
- **版本**: 1.2.42-local.b7199c5
- **Commit**: b7199c5
- **部署位置**: `~/.local/lib/codex-plus-plus/versions/1.2.42-local.b7199c5/`
- **进程 PID**: 90635

## 已安装版本

- `1.2.42` - 原始版本
- `1.2.42-local.624dd13` - 旧版本
- `1.2.42-local.8d6614f` - 第一次修复（自定义 GPT 模型支持）
- `1.2.42-local.b7199c5` - 当前版本（支持所有自定义模型）✨

## 现在支持的模型

### GPT 系列
- ✅ gpt-5.5, gpt-5.6-sol 等（使用 GPT 专用模板）
- ✅ gpt-4o, gpt-4-turbo, gpt-3.5-turbo 等（使用通用模板）

### Claude 系列
- ✅ claude-opus-4, claude-sonnet-4 等（使用通用模板）
- ✅ 不再被强制使用 GPT 的行为模式

### 其他模型
- ✅ Grok (xai-grok-2, grok-beta 等)
- ✅ Gemini (gemini-2.0-flash-exp 等)
- ✅ DeepSeek (使用 DeepSeek 专用模板)
- ✅ 任何其他自定义模型

## 如何验证

1. 打开 CodexPlusPlus Manager
2. 配置供应商（如 Claude、Grok 等）
3. 填写 Base URL 和 API Key
4. 在"模型"字段输入任意模型名称
5. 点击"测试连接"按钮
6. 应该能成功连接并识别模型
7. 启动 Codex 后，模型将按照其原生行为工作，而不是模仿 GPT

## GitHub 提交

- 第一次修复: https://github.com/MisakiSATA/CodexPlusPlus/commit/8d6614f
- 第二次修复: https://github.com/MisakiSATA/CodexPlusPlus/commit/b7199c5

## 相关文档

- `CUSTOM_MODEL_FIX.md` - 第一次修复的技术细节
- `DEPLOYMENT_STATUS.md` - 第一次部署状态

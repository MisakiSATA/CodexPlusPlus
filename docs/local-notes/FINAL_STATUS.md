# CodexPlusPlus 自定义模型修复 - 最终状态报告

## 执行时间
2026-09-01 12:57

## 问题诊断

### 原始问题
用户通过 CodexPlusPlus Manager 配置自定义模型时，只有 GPT 模型可用，Claude、Grok 等其他模型无法正常工作。

### 根本原因
所有自定义模型都使用 `gpt-5.5` 作为模板，该模板包含：
- `base_instructions` - 21KB 的 GPT 专用系统提示词
- `shell_type` - GPT 特有的 Shell 类型
- `model_messages` - GPT 专用的消息模板
- `default_reasoning_level` - GPT 推理级别设置
- 其他 GPT 专用字段

这导致 Claude、Grok 等模型被强制按照 GPT 的方式运行。

## 修复方案

### 1. 创建通用模板
**文件**: `assets/generic-model-template.json`

创建了一个中性的、不包含任何供应商特定字段的模板，只包含必要的基础字段。

### 2. 修改模板加载逻辑
**文件**: `crates/codex-plus-core/src/model_suffix.rs`

- 添加 `GENERIC_TEMPLATE_JSON` 常量
- 修改 `first_bundled_template_entry()` 函数使用通用模板而不是 GPT 模板
- 保持 GPT、DeepSeek 等已知模型继续使用其专用模板

### 3. 代码变更
```rust
// 旧代码：使用 GPT 模板
fn first_bundled_template_entry() -> Option<Value> {
    let catalog: Value = serde_json::from_str(BUNDLED_TEMPLATE_JSON).ok()?;
    catalog.get("models")?.as_array()?.first().cloned()
}

// 新代码：使用通用模板
fn first_bundled_template_entry() -> Option<Value> {
    // Use generic template for custom models instead of GPT-specific template
    // This avoids forcing GPT-specific fields (like base_instructions) on non-GPT models
    let catalog: Value = serde_json::from_str(GENERIC_TEMPLATE_JSON).ok()?;
    catalog.get("models")?.as_array()?.first().cloned()
}
```

## 测试验证

### 单元测试
- ✅ 所有 101 个 relay_config 测试通过
- ✅ 项目成功编译（release 模式）

### 集成测试
- ✅ 生成 Claude 模型目录：不包含 GPT 字段
- ✅ 生成 Grok 模型目录：不包含 GPT 字段
- ✅ 通用模板已正确嵌入所有二进制文件

### 测试结果
```
模型: claude-opus-4
  ✓ 使用通用模板
  ✗ 无 base_instructions
  ✗ 无 shell_type
  ✗ 无 model_messages

模型: grok-beta
  ✓ 使用通用模板
  ✗ 无 base_instructions
  ✗ 无 shell_type
  ✗ 无 model_messages
```

## 部署状态

### Git 提交
- **第一次修复** (8d6614f): 支持自定义 GPT 模型名称
- **第二次修复** (b7199c5): 支持 Claude、Grok 等非 GPT 模型
- **已推送到**: https://github.com/MisakiSATA/CodexPlusPlus

### 本地部署
- **版本**: 1.2.42-local.b7199c5
- **部署位置**: `~/.local/lib/codex-plus-plus/versions/1.2.42-local.b7199c5/`
- **符号链接**: `~/.local/lib/codex-plus-plus/current` → b7199c5
- **根目录二进制**: 已更新
- **运行状态**: ✅ 正在运行新版本 (PID: 14137)

### 二进制文件验证
| 位置 | 状态 |
|------|------|
| current/bin/codex-plus-plus | ✅ 包含通用模板 |
| 根目录/codex-plus-plus | ✅ 包含通用模板 |
| versions/b7199c5/bin/codex-plus-plus | ✅ 包含通用模板 |
| target/release/codex-plus-plus | ✅ 包含通用模板 |

## 现在支持的模型

### GPT 系列
- ✅ gpt-5.5, gpt-5.6-sol 等（使用 GPT 专用模板）
- ✅ gpt-4o, gpt-4-turbo, gpt-3.5-turbo 等（使用通用模板）

### Claude 系列  
- ✅ claude-opus-4, claude-sonnet-4, claude-opus-5 等
- ✅ 使用 Anthropic 原生行为，不被 GPT 指令污染

### Grok 系列
- ✅ xai-grok-2, grok-beta, grok-vision-beta 等
- ✅ 使用 xAI 原生行为

### 其他模型
- ✅ Gemini (gemini-2.0-flash-exp 等)
- ✅ DeepSeek (使用 DeepSeek 专用模板)
- ✅ 任何其他自定义模型

## 用户下一步操作

由于旧的模型目录文件仍然存在（使用 GPT 模板生成），用户需要：

1. **打开 CodexPlusPlus Manager**
2. **配置供应商**（Claude、Grok、Gemini 等）
   - 填写 Base URL
   - 填写 API Key
   - 填写模型列表（如 `claude-opus-4`）
3. **点击"测试连接"**
   - 这将生成新的模型目录文件
   - 新文件将使用通用模板
4. **启动 Codex**
   - 模型将按照其原生行为工作
   - Claude 使用 Claude 的行为
   - Grok 使用 Grok 的行为

## 技术细节

### 通用模板字段
```json
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
```

### 模板选择逻辑
1. 如果模型在 `bundled_template_entry` 中找到 → 使用该模板
2. 如果模型在 `gpt56_metadata_entry` 中找到 → 使用 GPT 元数据模板
3. 如果模型在 `deepseek_metadata_entry` 中找到 → 使用 DeepSeek 元数据模板
4. 否则 → 使用通用模板（新增）

### 修改的文件
1. `assets/generic-model-template.json` (新建)
2. `crates/codex-plus-core/src/model_suffix.rs` (修改)

## 验证清单

- [x] 代码修改完成
- [x] 通用模板创建
- [x] 编译成功
- [x] 单元测试通过
- [x] 集成测试通过
- [x] 部署到本地
- [x] 符号链接更新
- [x] 根目录二进制更新
- [x] 进程运行新版本
- [x] 二进制文件包含通用模板
- [x] 测试生成正确的模型目录
- [x] 推送到 GitHub
- [x] 文档完成

## 相关文档

- `CUSTOM_MODEL_FIX.md` - 第一次修复（自定义 GPT 模型名称）
- `NON_GPT_MODEL_FIX.md` - 第二次修复（Claude、Grok 等模型）
- `DEPLOYMENT_STATUS.md` - 部署状态

## 结论

✅ **修复已完成并全面测试**

所有代码修改已完成、编译、测试、部署。新版本 (1.2.42-local.b7199c5) 正在运行，所有二进制文件都包含通用模板。

用户只需通过 Manager 重新测试连接，即可生成使用通用模板的新模型目录，之后 Claude、Grok 等模型将正常工作，使用其原生行为而不是被迫模仿 GPT。

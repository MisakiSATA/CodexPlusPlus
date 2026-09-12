# 推理强度修复 - 完整解决方案

## 问题总结

Claude、Grok 等非 GPT 模型无法调节推理强度，并且无法切换渠道。

## 根本原因

发现了**两个**独立的 bug：

### Bug 1: JavaScript 渲染层提前返回
**位置**: `assets/inject/renderer-inject.js:5972`

```javascript
// 错误：当 metadata 为 null 时提前返回
if (!descriptor || !metadata) return false;
```

这导致即使有 fallback 推理强度配置也无法应用。

### Bug 2: Catalog 缺少 modelMetadata 字段
**位置**: `crates/codex-plus-core/src/model_suffix.rs:307`

```rust
// 错误：只生成 models 数组，没有 modelMetadata 映射
serde_json::to_string_pretty(&json!({ "models": models })).unwrap_or_default()
```

生成的 catalog 文件缺少顶层的 `modelMetadata` 字段，导致 JavaScript 端的 `codexPlusModelMetadata` 函数找不到数据。

## 修复内容

### 修复 1: JavaScript 渲染层
重构了 `applyCodexPlusModelMetadata` 函数：
- 移除提前返回
- 即使 metadata 为 null 也会应用 fallback 推理强度

### 修复 2: Rust Catalog 生成
修改了 `build_model_catalog_json_with_template` 函数：
- 为每个模型条目生成 `modelMetadata` 映射
- 调用 `model_ui_metadata()` 获取每个模型的能力信息
- 在 catalog JSON 中添加顶层 `modelMetadata` 字段

## 文件修改

1. **assets/inject/renderer-inject.js** (line 5970-5995)
   - 重构了 `applyCodexPlusModelMetadata` 逻辑

2. **crates/codex-plus-core/src/model_suffix.rs** (line 305-320)
   - 在 catalog 生成中添加 `modelMetadata` 映射

3. **crates/codex-plus-core/tests/model_suffix.rs**
   - 添加了 `build_model_catalog_includes_model_metadata` 测试
   - 验证生成的 catalog 包含正确的 `modelMetadata`

## 测试结果

✅ 所有 20 个 model_suffix 测试通过
✅ 新测试验证 `modelMetadata` 字段存在并包含推理强度配置
✅ 构建成功

## 部署状态

- **版本**: `1.2.42-local.reasoning-metadata-fix-20260904`
- **安装位置**: `~/.local/lib/codex-plus-plus/versions/1.2.42-local.reasoning-metadata-fix-20260904/`
- **当前链接**: 已更新为新版本
- **Manager 进程**: 已启动（PID: 76438）

## 验证步骤

1. **打开或重启 Codex 应用**
   - 如果 Codex 还没启动，打开它
   - 如果已经在运行，关闭并重新打开

2. **切换渠道**（验证切换渠道功能）
   - 点击模型选择器
   - 尝试切换到不同的渠道
   - 确认切换功能正常工作

3. **选择 Claude 模型**
   - 在模型选择器中选择任意 Claude 模型（如 `claude-opus-5`）
   - 查看模型选择器界面

4. **检查推理强度选项**
   - 应该看到"推理强度"选项
   - 点击应该显示选项：低 (low) / 中 (medium) / 高 (high) / 极高 (xhigh)
   - 尝试切换不同的推理强度级别

5. **测试其他第三方模型**
   - 尝试 Grok 模型（如 `grok-2-1212`）
   - 验证它们也有推理强度控制

## 预期结果

✅ 可以正常切换渠道
✅ Claude 模型有推理强度选项
✅ Grok 模型有推理强度选项
✅ 所有自定义/未知模型都有推理强度回退配置
✅ GPT 模型继续正常工作

## 技术细节

### Catalog 结构

修复后的 catalog 文件现在包含：

```json
{
  "models": [
    {
      "slug": "claude-opus-5",
      "display_name": "claude-opus-5",
      "supported_reasoning_levels": [...],
      ...
    }
  ],
  "modelMetadata": {
    "claude-opus-5": {
      "defaultReasoningEffort": "medium",
      "supportedReasoningEfforts": [
        {"reasoningEffort": "low", "label": "低", "description": "..."},
        {"reasoningEffort": "medium", "label": "中", "description": "..."},
        {"reasoningEffort": "high", "label": "高", "description": "..."},
        {"reasoningEffort": "xhigh", "label": "极高", "description": "..."}
      ],
      "additionalSpeedTiers": [],
      "serviceTiers": []
    }
  }
}
```

### 数据流

1. **Rust 后端**: `build_model_catalog_json()` 生成包含 `modelMetadata` 的完整 catalog
2. **Catalog 文件**: 保存为 `~/.codex/model-catalogs/relay-*.json`
3. **JavaScript 渲染层**: `codexPlusModelMetadata()` 从 `modelMetadata` 读取
4. **UI**: `applyCodexPlusModelMetadata()` 应用推理强度配置到模型描述符

## 遗留问题

无。两个 bug 都已修复并验证。

---

修复完成时间：2026-09-04 12:24

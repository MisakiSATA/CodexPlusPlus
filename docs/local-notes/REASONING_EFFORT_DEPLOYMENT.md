# Reasoning Effort Fix Deployment - 2026-09-04

## 问题
只有 GPT 模型可以调节推理强度，Claude、Grok 等第三方模型无法调节推理强度。

## 修复内容
修复了 `assets/inject/renderer-inject.js` 中 `applyCodexPlusModelMetadata` 函数的早期返回 bug。

### 修复前（错误的代码）
```javascript
function applyCodexPlusModelMetadata(descriptor, modelName) {
  const metadata = codexPlusModelMetadata(modelName);
  if (!descriptor || !metadata) return false;  // ← Bug: metadata 为 null 时提前返回
  // 后续代码永远不会执行
}
```

### 修复后（正确的代码）
```javascript
function applyCodexPlusModelMetadata(descriptor, modelName) {
  if (!descriptor) return false;
  const metadata = codexPlusModelMetadata(modelName);
  let changed = false;

  // Apply metadata fields when available
  if (metadata) {
    for (const key of ["displayName", "description", "defaultReasoningEffort"]) {
      if (typeof metadata[key] === "string" && metadata[key] && descriptor[key] !== metadata[key]) {
        descriptor[key] = metadata[key];
        changed = true;
      }
    }
  }

  // Always ensure reasoning efforts are present (uses fallback when metadata is null)
  const nextEfforts = modelReasoningEfforts(modelName);
  if (JSON.stringify(descriptor.supportedReasoningEfforts || []) !== JSON.stringify(nextEfforts)) {
    descriptor.supportedReasoningEfforts = nextEfforts;
    changed = true;
  }

  return changed;
}
```

## 部署信息

### 版本
- 版本号：`1.2.42-local.reasoning-effort-fix-20260904`
- 基于 commit：`84e3c3a`
- 部署时间：2026-09-04

### 部署路径
```
~/.local/lib/codex-plus-plus/versions/1.2.42-local.reasoning-effort-fix-20260904/
├── bin/
│   ├── codex-plus-plus
│   └── codex-plus-plus-manager
```

当前版本符号链接已更新：
```
~/.local/lib/codex-plus-plus/versions/current -> 1.2.42-local.reasoning-effort-fix-20260904
```

## 验证步骤

1. 确认新版本已部署：
```bash
readlink ~/.local/lib/codex-plus-plus/versions/current
```

2. 验证修复代码存在于二进制中：
```bash
strings ~/.local/lib/codex-plus-plus/versions/current/bin/codex-plus-plus | \
  grep -A 10 "Always ensure reasoning efforts are present"
```

3. 重启 Codex：
```bash
~/.local/lib/codex-plus-plus/versions/current/bin/codex-plus-plus-manager restart
```

4. 测试非 GPT 模型的推理强度控制：
   - 切换到 Claude 模型（如 claude-opus-5）
   - 打开模型设置
   - 确认"推理强度"选项可用，有 low/medium/high/xhigh 选项

## 预期结果

修复后，所有模型都应该有推理强度控制：
- ✅ GPT 模型：继续使用特定的元数据
- ✅ Claude 模型：使用通用回退（low/medium/high/xhigh）
- ✅ Grok 模型：使用通用回退
- ✅ 所有自定义模型：使用通用回退

## 技术细节

见 `REASONING_EFFORT_FIX.md` 获取完整的技术文档。

## 注意事项

- 之前的 `1.2.42-local.custom-model-reasoning-fix-20260903` 版本并未真正修复此问题
- 本次部署是完整修复版本
- 需要重启 Codex 才能加载新的 renderer-inject.js 代码

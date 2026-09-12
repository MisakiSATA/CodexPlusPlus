# Catalog 缓存刷新修复

## 问题描述

用户报告了三个相关问题：

1. **切换渠道经常超时** - 切换后前端等待新 catalog 但使用了旧缓存
2. **编辑渠道会很卡** - 保存配置后 catalog 没有及时更新
3. **编辑完成后再次进入推理强度又不能更改了** - UI 使用了旧的 catalog 缓存

## 根本原因

### 1. Catalog 缓存逻辑（第 5910 行）

```javascript
if (!force && codexModelCatalogLoadedAt && Date.now() - codexModelCatalogLoadedAt < 10000) 
    return codexModelCatalog;
```

在 10 秒内阻止重新加载 catalog，即使配置已更新。

### 2. 设置保存后没有清除缓存

`setBackendSetting` 函数（第 3013-3026 行）在保存设置后：
- ✅ 调用后端 `/settings/set` API
- ✅ 更新本地设置对象
- ❌ **没有清除 catalog 缓存**
- ❌ **没有强制重新加载 catalog**

结果：用户切换渠道或编辑配置后，前端继续使用旧的 catalog（最多 10 秒），导致：
- 推理强度控件显示旧模型的配置
- 模型列表不更新
- UI 卡顿（可能在重试加载）

## 修复方案

在 `setBackendSetting` 函数中，设置保存成功后：

1. 清除 catalog 缓存时间戳：`codexModelCatalogLoadedAt = 0`
2. 强制重新加载 catalog：`await loadCodexModelCatalog(true)`

### 修改的代码

```javascript
async function setBackendSetting(key, value) {
  const seq = ++codexPlusBackendSettingsSeq;
  codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
  codexPlusBackendSettingsLoaded = true;
  refreshCodexPlusBackendToggles();
  try {
    const settings = await postJson("/settings/set", { [key]: value });
    if (seq === codexPlusBackendSettingsSeq) {
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
    }
    // 清除 catalog 缓存，强制重新加载以获取更新后的模型元数据
    codexModelCatalogLoadedAt = 0;
    await loadCodexModelCatalog(true);
  } finally {
    refreshCodexPlusBackendToggles();
  }
}
```

## 测试计划

### 测试 1: 切换渠道后推理强度立即可用

1. 打开 Codex Manager，切换到 GPT 渠道
2. 打开 Codex UI，验证推理强度控件存在且可用
3. 切换到 Claude 渠道
4. **立即**打开模型选择器，验证推理强度控件存在且可用（不需要等待 10 秒）

### 测试 2: 编辑配置后立即生效

1. 打开 Manager，编辑当前渠道的 `model_list`（添加一个新模型）
2. 保存配置
3. **立即**打开 Codex UI 的模型选择器
4. 验证新模型出现在列表中，且有推理强度控件

### 测试 3: 快速连续切换不卡顿

1. 在 Manager 中快速切换 GPT → Claude → Grok → GPT
2. 每次切换后立即检查 UI
3. 验证没有超时错误，没有卡顿

## 受影响的文件

- `assets/inject/renderer-inject.js` - 主修复
- （无需修改后端，后端已正确生成 catalog）

## 部署步骤

1. 编译：`cargo build --release`
2. 部署二进制文件
3. 重启 Codex
4. 验证上述测试场景

## 相关修复

这个修复与之前的推理强度修复互补：

- **之前修复**：`applyCodexPlusModelMetadata` 不再提前返回，确保第三方模型获得 fallback 推理强度
- **本次修复**：确保配置更改后 catalog 立即刷新，让 UI 获得最新的模型元数据

两个修复共同确保了用户体验的流畅性。

# Catalog 缓存刷新修复验证指南

## 修复内容

修复了 `renderer-inject.js` 中的 catalog 缓存问题：
- 在 `setBackendSetting` 函数中，设置保存成功后强制刷新 catalog 缓存
- 解决了切换渠道/编辑配置后推理强度控件失效的问题

## 部署状态

✅ 编译完成：2026-09-04 12:57
✅ 二进制部署：`~/.codex/version/current/codex-plus-plus`
✅ Codex 重启：PID 101073
✅ Catalog 更新：`~/.codex/model-catalogs/relay-mtcdgytu.json`

## 手动测试步骤

### 测试 1：切换渠道后推理强度立即可用

**步骤：**
1. 打开 Codex Manager（通常在系统托盘或菜单栏）
2. 当前渠道切换到 **GPT** 渠道
3. 打开 Codex UI 网页（http://localhost:端口）
4. 点击模型选择器
5. 验证推理强度控件存在且可用
6. 在 Manager 中切换到 **Claude** 渠道
7. **立即**（不要等待）刷新 Codex UI 或重新打开模型选择器
8. 验证推理强度控件存在且可用

**预期结果：**
- ✅ 切换后立即可见推理强度控件（low/medium/high/xhigh）
- ✅ 不需要等待 10 秒
- ✅ 没有超时错误
- ✅ 界面不卡顿

**旧版本问题：**
- ❌ 切换后推理强度控件消失或显示旧配置
- ❌ 需要等待 10 秒才能看到新配置
- ❌ 界面卡顿或显示加载状态

---

### 测试 2：编辑配置后立即生效

**步骤：**
1. 打开 Codex Manager
2. 点击当前渠道的"编辑"或"设置"
3. 修改 `model_list`，例如添加一个新模型：
   ```
   claude-opus-5
   claude-sonnet-5
   grok-3
   ```
4. 保存配置
5. **立即**（不要等待）打开 Codex UI
6. 打开模型选择器
7. 验证新添加的模型出现在列表中
8. 验证新模型有推理强度控件

**预期结果：**
- ✅ 新模型立即出现在列表中
- ✅ 新模型有推理强度控件
- ✅ 保存配置后界面响应迅速
- ✅ 没有卡顿或超时

**旧版本问题：**
- ❌ 保存后新模型不出现，需要等待 10 秒
- ❌ 界面卡顿或显示加载中
- ❌ 推理强度控件失效

---

### 测试 3：快速连续切换不卡顿

**步骤：**
1. 打开 Codex Manager 和 Codex UI
2. 快速切换渠道：GPT → Claude → Grok → Deepseek → GPT
3. 每次切换后立即检查 UI 的模型选择器
4. 验证每次都能看到正确的模型列表和推理强度控件

**预期结果：**
- ✅ 每次切换后立即显示正确的模型列表
- ✅ 推理强度控件始终可用
- ✅ 界面响应流畅，无卡顿
- ✅ 没有错误或超时提示

**旧版本问题：**
- ❌ 快速切换导致界面卡死
- ❌ 模型列表显示混乱
- ❌ 推理强度控件时有时无

---

## 技术验证

### 验证 Catalog 包含 modelMetadata

```bash
# 检查当前 catalog 是否有 modelMetadata 字段
jq '.modelMetadata' ~/.codex/model-catalogs/relay-*.json | head -20
```

**预期输出：**
```json
{
  "claude-opus-5": {
    "defaultReasoningEffort": "medium",
    "supportedReasoningEfforts": [
      {"effort": "low", "label": "Low"},
      {"effort": "medium", "label": "Medium"},
      {"effort": "high", "label": "High"},
      {"effort": "xhigh", "label": "Very High"}
    ],
    ...
  }
}
```

### 验证缓存刷新逻辑

在浏览器开发者工具中：
1. 打开 Console
2. 切换渠道
3. 观察网络请求
4. 应该能看到 `/api/v1/models` 请求立即发出

---

## 问题排查

### 如果推理强度控件仍然不出现

1. **清除浏览器缓存**
   - 强制刷新：Ctrl+Shift+R (Linux/Windows) 或 Cmd+Shift+R (Mac)
   - 或者清除浏览器缓存后重新打开

2. **检查 catalog 文件**
   ```bash
   # 查看最新的 catalog
   ls -lt ~/.codex/model-catalogs/
   
   # 检查是否包含 modelMetadata
   cat ~/.codex/model-catalogs/relay-*.json | jq '.modelMetadata | keys'
   ```

3. **检查 Codex 日志**
   ```bash
   # 查看运行日志
   journalctl -u codex-plus-plus -f
   # 或
   tail -f /tmp/codex-plus-plus.log
   ```

4. **重启 Codex**
   ```bash
   pkill codex-plus-plus
   nohup ~/.codex/version/current/codex-plus-plus > /tmp/codex-plus-plus.log 2>&1 &
   ```

---

## 相关修复

这是第二个相关修复，与之前的修复配合使用：

### 修复 1：推理强度 fallback（已完成）
- **文件**：`assets/inject/renderer-inject.js` 第 5972 行
- **问题**：`applyCodexPlusModelMetadata` 提前返回，导致第三方模型没有推理强度
- **修复**：移除提前返回，始终应用 fallback 推理强度

### 修复 2：Catalog 缓存刷新（本次修复）
- **文件**：`assets/inject/renderer-inject.js` 第 3013-3026 行
- **问题**：设置保存后 catalog 缓存不刷新，导致 UI 显示旧数据
- **修复**：设置保存后强制刷新 catalog 缓存

两个修复共同确保：
1. 所有模型（GPT、Claude、Grok 等）都有推理强度控件
2. 配置更改后立即生效，无需等待
3. 界面响应流畅，无卡顿

---

## 提交信息

```
fix: force catalog reload after settings change to prevent stale UI

- Clear catalog cache timestamp after setBackendSetting succeeds
- Call loadCodexModelCatalog(true) to force immediate reload
- Fixes reasoning effort controls disappearing after channel switch
- Fixes UI lag when editing channel configuration
- Fixes timeout errors during rapid channel switching

The catalog cache (10s TTL) was preventing immediate UI updates when
users switched channels or edited configuration. Now the cache is
invalidated and reloaded whenever backend settings change.
```

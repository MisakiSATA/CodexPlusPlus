# 推理强度修复验证指南

## 如何验证修复已生效

### 1. 确认版本已更新
```bash
readlink ~/.local/lib/codex-plus-plus/versions/current
# 应该显示: 1.2.42-local.reasoning-effort-fix-20260904
```

### 2. 在 Codex 界面中测试

#### 测试 Claude 模型
1. 打开 Codex
2. 点击左上角的"模型"下拉菜单
3. 选择一个 Claude 模型（如 `claude-opus-5`）
4. 再次点击"模型"区域，应该看到：
   - **模型**: claude-opus-5
   - **推理强度**: 中 （或 low/medium/high/xhigh 选项）

#### 测试 Grok 模型
1. 切换到 Grok 模型
2. 同样应该能看到"推理强度"选项

#### 测试 GPT 模型（确保没有破坏原有功能）
1. 切换到 GPT 模型（如 `gpt-5.6-sol`）
2. 应该正常显示推理强度选项

### 3. 预期结果

修复前：
- ✗ Claude 模型：无"推理强度"选项
- ✗ Grok 模型：无"推理强度"选项  
- ✓ GPT 模型：有推理强度选项

修复后：
- ✓ Claude 模型：有推理强度选项（low/medium/high/xhigh）
- ✓ Grok 模型：有推理强度选项（low/medium/high/xhigh）
- ✓ GPT 模型：继续正常工作

### 4. 如果仍然没有显示

1. 确保已重启 Codex：
   ```bash
   ~/.local/lib/codex-plus-plus/versions/current/bin/codex-plus-plus-manager restart
   ```

2. 检查是否使用了正确的版本：
   ```bash
   strings ~/.local/lib/codex-plus-plus/versions/current/bin/codex-plus-plus | \
     grep -A 5 "Always ensure reasoning efforts"
   ```
   应该能看到修复后的代码。

3. 清除缓存（如果需要）：
   关闭 Codex，删除缓存，重新启动

### 5. 截图示例

修复后应该看到类似你截图中显示的界面：
```
模型              claude-opus-5  >
推理强度          中              >
```

点击"推理强度"应该能看到：
- 低 (low)
- 中 (medium) 
- 高 (high)
- 超高 (xhigh)

## 技术验证

如果想从技术层面验证，可以检查浏览器控制台：

1. 打开 Codex 开发者工具（如果可用）
2. 在控制台输入：
   ```javascript
   // 获取当前模型的元数据
   codexModelCatalog
   ```
3. 检查 `modelMetadata` 对象，应该包含所有模型的 `supportedReasoningEfforts`

## 日期
2026-09-04

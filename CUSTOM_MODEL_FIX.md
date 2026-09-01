# 自定义 GPT 模型支持修复

## 问题描述

在修复之前，CodexPlusPlus 对 GPT 模型的自定义模型名称支持存在限制：
- Claude 和其他模型可以使用任意自定义模型名称
- GPT 模型只能使用预定义的模型（如 gpt-5.5、gpt-5.6-sol 等）
- 无法使用自定义的 GPT 模型名称（如 gpt-4o、gpt-4-turbo、gpt-3.5-turbo 等）

## 根本原因

问题出在 `crates/codex-plus-core/src/relay_config.rs` 中的模型目录生成逻辑。

原代码在第 1541-1546 行：
```rust
// Known bundled metadata entries need a catalog even without a user-supplied window.
if !entries.iter().any(|entry| {
    entry.suffix_window.is_some()
        || crate::model_suffix::requires_bundled_metadata_catalog(&entry.slug)
}) {
    return Ok(config_text.to_string());
}
```

这段代码只在以下情况下生成 `model_catalog_json`：
1. 模型名称带有窗口后缀（如 `model-name[1M]`）
2. 模型是已知的内置模型（GPT-5.6 系列、DeepSeek 等）

对于普通的自定义 GPT 模型名称，如果没有窗口后缀，就不会生成模型目录，导致 Claude Code 无法识别这些模型。

## 解决方案

修改了检查逻辑，允许为所有自定义模型生成模型目录：

```rust
// Always generate catalog for custom models to support arbitrary model names.
// Skip only when no models are defined at all.
if entries.is_empty() {
    return Ok(config_text.to_string());
}
```

现在只要模型列表不为空，就会生成模型目录，支持任意自定义模型名称。

## 修改的文件

1. **crates/codex-plus-core/src/relay_config.rs** (第 1538-1547 行)
   - 移除了限制性检查
   - 改为只检查模型列表是否为空

2. **crates/codex-plus-core/tests/relay_config.rs** (3 处测试更新)
   - `apply_relay_profile_does_not_write_model_catalog_json_for_selected_models`
   - `apply_relay_profile_does_not_carry_previous_managed_model_catalog`
   - `apply_relay_profile_no_catalog_when_model_list_has_no_suffix`
   - 更新这些测试以反映新的行为（现在会为所有模型生成目录）

## 测试验证

- ✅ 所有 101 个 relay_config 测试通过
- ✅ 项目成功编译（release 模式）
- ✅ 不影响现有功能

## 使用方法

修复后，用户现在可以：
1. 在 GPT 供应商配置中使用任意模型名称
2. 例如：`gpt-4o`、`gpt-4-turbo`、`gpt-3.5-turbo`、`gpt-4o-mini` 等
3. 测试连接功能将正常工作
4. 模型将在 Claude Code 中正确显示和使用

## 影响范围

此修复仅影响模型目录生成逻辑，不影响：
- 现有的 GPT-5.6 系列模型支持
- DeepSeek 等其他供应商
- 模型窗口后缀功能
- 用户自定义的 model_catalog_json 文件

## 日期

2026-09-01

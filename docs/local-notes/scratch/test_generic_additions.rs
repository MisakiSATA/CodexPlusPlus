#[cfg(test)]
mod test_generic_template {
    use super::*;

    #[test]
    fn test_generic_template_has_no_gpt_fields() {
        let template = first_bundled_template_entry().expect("Should load generic template");

        // 验证通用模板不包含 GPT 专用字段
        assert!(!template.get("base_instructions").is_some(),
                "Generic template should not have base_instructions");
        assert!(!template.get("shell_type").is_some(),
                "Generic template should not have shell_type");
        assert!(!template.get("model_messages").is_some(),
                "Generic template should not have model_messages");
        assert!(!template.get("default_reasoning_level").is_some(),
                "Generic template should not have default_reasoning_level");

        // 验证通用模板包含必要字段
        assert_eq!(template.get("slug").and_then(Value::as_str), Some("generic-custom-model"));
        assert_eq!(template.get("supported_in_api").and_then(Value::as_bool), Some(true));
        assert_eq!(template.get("visibility").and_then(Value::as_str), Some("list"));
    }

    #[test]
    fn test_build_catalog_for_claude_model() {
        let entries = vec![
            ModelCatalogEntry {
                slug: "claude-opus-4".to_string(),
                display_name: "Claude Opus 4".to_string(),
                suffix_window: Some(200000),
            }
        ];

        let catalog_json = build_model_catalog_json(&entries, None);
        let catalog: Value = serde_json::from_str(&catalog_json).expect("Should parse catalog");

        let models = catalog.get("models").and_then(Value::as_array).expect("Should have models array");
        assert_eq!(models.len(), 1);

        let claude = &models[0];
        assert_eq!(claude.get("slug").and_then(Value::as_str), Some("claude-opus-4"));
        assert_eq!(claude.get("context_window").and_then(Value::as_u64), Some(200000));

        // 验证 Claude 模型不包含 GPT 专用字段
        assert!(!claude.get("base_instructions").is_some(),
                "Claude model should not have base_instructions from GPT template");
        assert!(!claude.get("shell_type").is_some(),
                "Claude model should not have shell_type from GPT template");
    }
}

use std::collections::HashMap;

use codex_plus_core::model_suffix::{
    ModelCatalogEntry, build_model_catalog_json, build_model_catalog_json_with_template,
    collect_catalog_entries, model_ui_metadata, parse_model_suffix,
};
use serde_json::Value;

#[test]
fn parse_suffix_extracts_k_and_m_units() {
    assert_eq!(
        parse_model_suffix("deepseek-v4-pro[1M]"),
        ("deepseek-v4-pro".to_string(), Some(1_000_000))
    );
    assert_eq!(
        parse_model_suffix("claude-sonnet-4[200K]"),
        ("claude-sonnet-4".to_string(), Some(200_000))
    );
    assert_eq!(
        parse_model_suffix("gpt-5.5[512k]"),
        ("gpt-5.5".to_string(), Some(512_000))
    );
    assert_eq!(
        parse_model_suffix("gpt-5.5[1000000]"),
        ("gpt-5.5".to_string(), Some(1_000_000))
    );
}

#[test]
fn parse_suffix_returns_none_without_bracket() {
    assert_eq!(parse_model_suffix("gpt-5.5"), ("gpt-5.5".to_string(), None));
    assert_eq!(
        parse_model_suffix("  qwen3-coder  "),
        ("qwen3-coder".to_string(), None)
    );
}

#[test]
fn parse_suffix_keeps_original_slug_when_bracket_invalid() {
    // 括号内非合法窗口 token 时，整串（含括号）作为 slug，window=None
    let (slug, window) = parse_model_suffix("foo[bar]");
    assert_eq!(slug, "foo[bar]");
    assert_eq!(window, None);

    // 括号未闭合：不剥离
    let (slug2, window2) = parse_model_suffix("foo[1M");
    assert_eq!(slug2, "foo[1M");
    assert_eq!(window2, None);
}

#[test]
fn parse_suffix_rejects_zero_and_negative() {
    assert_eq!(parse_model_suffix("foo[0K]"), ("foo[0K]".to_string(), None));
}

#[test]
fn collect_entries_includes_current_model_and_strips_suffix() {
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    let entries = collect_catalog_entries(
        "deepseek-v4-pro\nqwen3-coder",
        &windows,
        &HashMap::new(),
        "deepseek-v4-pro",
    );
    // 当前 model 与列表去重后共 2 条
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "deepseek-v4-pro");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
    assert_eq!(entries[1].slug, "qwen3-coder");
    assert_eq!(entries[1].suffix_window, None);
}

#[test]
fn collect_entries_deduplicates() {
    let entries = collect_catalog_entries(
        "qwen3-coder\nqwen3-coder",
        &HashMap::new(),
        &HashMap::new(),
        "qwen3-coder",
    );
    assert_eq!(entries.len(), 1);
}

#[test]
fn build_catalog_json_writes_context_window_and_strips_suffix() {
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    windows.insert("claude-sonnet-4".to_string(), "200K".to_string());
    let entries = collect_catalog_entries(
        "deepseek-v4-pro\nclaude-sonnet-4",
        &windows,
        &HashMap::new(),
        "",
    );
    let catalog = build_model_catalog_json(&entries, None);
    assert!(catalog.contains(r#""slug": "deepseek-v4-pro""#));
    assert!(catalog.contains(r#""context_window": 1000000"#));
    assert!(catalog.contains(r#""max_context_window": 1000000"#));
    assert!(catalog.contains(r#""slug": "claude-sonnet-4""#));
    assert!(catalog.contains(r#""context_window": 200000"#));
    // 后缀不得进入 catalog
    assert!(!catalog.contains("[1M]"));
    assert!(!catalog.contains("[200K]"));
    // auto_compact 留 null（codex 按比例算）
    assert!(catalog.contains(r#""auto_compact_token_limit": null"#));
}

#[test]
fn build_catalog_json_uses_fallback_for_no_suffix_entries() {
    let entries = collect_catalog_entries("qwen3-coder", &HashMap::new(), &HashMap::new(), "");
    let catalog = build_model_catalog_json(&entries, Some(272_000));
    assert!(catalog.contains(r#""slug": "qwen3-coder""#));
    assert!(catalog.contains(r#""context_window": 272000"#));
}

#[test]
fn build_catalog_json_uses_runtime_compatible_generic_metadata() {
    let entries = collect_catalog_entries(
        "claude-opus-5\ngrok-4.5",
        &HashMap::new(),
        &HashMap::new(),
        "claude-opus-5",
    );
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();

    for model in catalog["models"].as_array().unwrap() {
        // 自定义模型现在有 4 档默认推理强度（low/medium/high/xhigh），不再是空数组
        let efforts = model["supported_reasoning_levels"]
            .as_array()
            .expect("supported_reasoning_levels should be an array")
            .iter()
            .filter_map(|entry| entry["effort"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(efforts, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(model["default_reasoning_level"], "medium");
        assert_eq!(model["shell_type"], "shell_command");
        assert_eq!(model["support_verbosity"], false);
        assert_eq!(
            model["truncation_policy"],
            serde_json::json!({ "mode": "tokens", "limit": 10_000 })
        );
        assert_eq!(model["experimental_supported_tools"], serde_json::json!([]));

        let instructions = model["base_instructions"]
            .as_str()
            .expect("generic model metadata must provide neutral base instructions");
        assert!(instructions.starts_with("You are Codex, a coding agent."));
        assert!(!instructions.contains("GPT"));
        assert!(model.get("model_messages").is_none());
    }
}

#[test]
fn build_catalog_json_uses_runtime_compatible_gpt56_metadata() {
    let entries = collect_catalog_entries(
        "gpt-5.6-sol\ngpt-5.6-terra\ngpt-5.6-luna",
        &HashMap::new(),
        &HashMap::new(),
        "gpt-5.6-sol",
    );
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let models = catalog["models"].as_array().unwrap();

    for (slug, default_reasoning, expected_efforts) in [
        (
            "gpt-5.6-sol",
            "low",
            vec!["low", "medium", "high", "xhigh", "max", "ultra"],
        ),
        (
            "gpt-5.6-terra",
            "medium",
            vec!["low", "medium", "high", "xhigh", "max", "ultra"],
        ),
        (
            "gpt-5.6-luna",
            "medium",
            vec!["low", "medium", "high", "xhigh", "max"],
        ),
    ] {
        let model = models.iter().find(|model| model["slug"] == slug).unwrap();
        let efforts = model["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["effort"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(model["context_window"], 272_000);
        assert_eq!(model["max_context_window"], 272_000);
        assert_eq!(model["default_reasoning_level"], default_reasoning);
        assert_eq!(efforts, expected_efforts);
        assert!(!efforts.contains(&"minimal"));
        assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
        assert_eq!(model["service_tiers"][0]["id"], "priority");
        assert_eq!(model["supports_search_tool"], true);
        assert_eq!(model["use_responses_lite"], true);
    }
}

#[test]
fn build_catalog_json_preserves_template_responses_lite_behavior() {
    let entries = collect_catalog_entries(
        "official-model",
        &HashMap::new(),
        &HashMap::new(),
        "official-model",
    );
    let template = serde_json::json!({
        "slug": "official-template",
        "supports_search_tool": true,
        "use_responses_lite": true
    });
    let catalog: serde_json::Value = serde_json::from_str(&build_model_catalog_json_with_template(
        &entries,
        None,
        Some(&template),
    ))
    .unwrap();

    assert_eq!(catalog["models"][0]["use_responses_lite"], true);
    assert_eq!(catalog["models"][0]["supports_search_tool"], true);
}

#[test]
fn astra_metadata_exposes_max_ultra_in_catalog_and_ui() {
    use codex_plus_core::model_suffix::requires_bundled_metadata_catalog;

    assert!(requires_bundled_metadata_catalog("gpt-6-astra"));
    assert!(!requires_bundled_metadata_catalog("gpt-6-astra-custom"));
    // 未知 slug 回落到 generic 模板：只给推理强度，不带 displayName（本 fork 行为）。
    let fallback = model_ui_metadata("gpt-6-astra-custom").expect("generic fallback metadata");
    assert!(fallback.get("displayName").is_none());
    assert!(
        !fallback["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let entries = collect_catalog_entries("gpt-6-astra", &HashMap::new(), &HashMap::new(), "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let model = &catalog["models"][0];
    let metadata = model_ui_metadata("gpt-6-astra").unwrap();
    let expected = vec!["low", "medium", "high", "xhigh", "max", "ultra"];
    for (levels, key) in [
        (&model["supported_reasoning_levels"], "effort"),
        (&metadata["supportedReasoningEfforts"], "reasoningEffort"),
    ] {
        let efforts: Vec<_> = levels
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level[key].as_str().unwrap())
            .collect();
        assert_eq!(efforts, expected);
    }
    assert_eq!(model["display_name"], "GPT-6-Astra");
    assert_eq!(metadata["displayName"], model["display_name"]);
    assert_eq!(model["default_reasoning_level"], "medium");
    assert_eq!(metadata["defaultReasoningEffort"], "medium");
    assert_eq!(model["context_window"], 272_000);
    assert_eq!(model["max_context_window"], 272_000);
    assert_eq!(model["supports_search_tool"], true);
    assert_eq!(model["supports_image_detail_original"], true);
    assert_eq!(model["use_responses_lite"], false);
    assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
    assert_eq!(
        metadata["additionalSpeedTiers"],
        model["additional_speed_tiers"]
    );
    assert_eq!(model["service_tiers"][0]["id"], "priority");
    assert_eq!(model["service_tiers"][0]["name"], "Fast");
    assert_eq!(metadata["serviceTiers"], model["service_tiers"]);

    let overridden: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, Some(200_000))).unwrap();
    assert_eq!(overridden["models"][0]["context_window"], 200_000);
}

#[test]
fn model_ui_metadata_exposes_fast_service_tier_capability() {
    let metadata = model_ui_metadata("gpt-5.6-sol").expect("Sol metadata should exist");

    assert_eq!(
        metadata["additionalSpeedTiers"],
        serde_json::json!(["fast"])
    );
    assert_eq!(metadata["serviceTiers"][0]["id"], "priority");
}

#[test]
fn model_ui_metadata_provides_generic_fallback_for_unknown_models() {
    // 未知的第三方模型（Claude、Grok 等）应该回落到 generic 推理能力
    for slug in ["claude-opus-5", "grok-4.5", "qwen-max", "some-random-model"] {
        let metadata = model_ui_metadata(slug)
            .unwrap_or_else(|| panic!("{slug} should get generic fallback metadata"));

        // 推理强度配置
        assert_eq!(metadata["defaultReasoningEffort"], "medium");
        let efforts = metadata["supportedReasoningEfforts"]
            .as_array()
            .expect("supportedReasoningEfforts should be an array")
            .iter()
            .filter_map(|entry| entry["reasoningEffort"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(efforts, vec!["low", "medium", "high", "xhigh"]);

        // 泛型回落不应该带 displayName，避免把所有模型改名为 "Custom Model"
        assert!(
            metadata.get("displayName").is_none(),
            "{slug} generic fallback should not override displayName"
        );
        assert!(
            metadata.get("description").is_none(),
            "{slug} generic fallback should not override description"
        );

        // 速度层级和服务层级空数组
        assert_eq!(metadata["additionalSpeedTiers"], serde_json::json!([]));
        assert_eq!(metadata["serviceTiers"], serde_json::json!([]));
    }
}

#[test]
fn build_model_catalog_includes_model_metadata() {
    let entries = vec![
        ModelCatalogEntry {
            slug: "claude-opus-5".to_string(),
            display_name: "claude-opus-5".to_string(),
            suffix_window: None,
            auto_compact_percent: None,
        },
        ModelCatalogEntry {
            slug: "grok-2-1212".to_string(),
            display_name: "grok-2-1212".to_string(),
            suffix_window: None,
            auto_compact_percent: None,
        },
    ];

    let catalog_json = build_model_catalog_json(&entries, None);
    let catalog: Value = serde_json::from_str(&catalog_json).unwrap();

    // 调试：打印 catalog 的顶层键
    eprintln!(
        "Catalog keys: {:?}",
        catalog.as_object().unwrap().keys().collect::<Vec<_>>()
    );
    eprintln!(
        "Has modelMetadata: {}",
        catalog.get("modelMetadata").is_some()
    );

    // 验证 modelMetadata 字段存在
    assert!(
        catalog.get("modelMetadata").is_some(),
        "modelMetadata field should exist"
    );

    let metadata = catalog["modelMetadata"].as_object().unwrap();

    // 验证所有模型都有 metadata
    assert!(
        metadata.contains_key("claude-opus-5"),
        "claude-opus-5 should have metadata"
    );
    assert!(
        metadata.contains_key("grok-2-1212"),
        "grok-2-1212 should have metadata"
    );

    // 验证 claude-opus-5 的 metadata 包含推理强度
    let claude_meta = &metadata["claude-opus-5"];
    assert!(
        claude_meta.get("supportedReasoningEfforts").is_some(),
        "claude-opus-5 should have supportedReasoningEfforts"
    );
    assert!(
        claude_meta.get("defaultReasoningEffort").is_some(),
        "claude-opus-5 should have defaultReasoningEffort"
    );

    // 验证推理强度数据正确
    let efforts = claude_meta["supportedReasoningEfforts"].as_array().unwrap();
    assert_eq!(efforts.len(), 4, "Should have 4 reasoning levels");
    assert_eq!(claude_meta["defaultReasoningEffort"], "medium");
}

#[test]
fn deepseek_metadata_yields_to_explicit_window_and_fallback() {
    let fallback_entries =
        collect_catalog_entries("deepseek-v4-pro", &HashMap::new(), &HashMap::new(), "");
    let fallback: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&fallback_entries, Some(512_000))).unwrap();
    assert_eq!(fallback["models"][0]["context_window"], 512_000);

    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "200K".to_string());
    let explicit_entries =
        collect_catalog_entries("deepseek-v4-pro", &windows, &HashMap::new(), "");
    let explicit: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&explicit_entries, Some(512_000))).unwrap();
    assert_eq!(explicit["models"][0]["context_window"], 200_000);
}

#[test]
fn collect_entries_adopts_suffix_for_current_model_from_list() {
    // 当前 model 本身无后缀，但 model_list 中靠后位置有同名带后缀条目。
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    let entries = collect_catalog_entries(
        "qwen3-coder\ndeepseek-v4-pro",
        &windows,
        &HashMap::new(),
        "deepseek-v4-pro",
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "deepseek-v4-pro");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
}

#[test]
fn collect_entries_prefers_later_suffix_for_duplicate_slug() {
    // 同一 slug 先出现无后缀条目，后出现带后缀条目，应采纳后者窗口。
    let mut windows = HashMap::new();
    windows.insert("deepseek/deepseek-v4-flash".to_string(), "1M".to_string());
    let entries = collect_catalog_entries(
        "deepseek/deepseek-v4-flash\ndeepseek/deepseek-v4-flash",
        &windows,
        &HashMap::new(),
        "",
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slug, "deepseek/deepseek-v4-flash");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
}

#[test]
fn collect_entries_prefers_later_suffix_when_reversed() {
    // 同一 slug 先出现 [1M]，后出现 [200K]，后者应覆盖前者。
    let mut windows = HashMap::new();
    windows.insert("deepseek/deepseek-v4-flash".to_string(), "200K".to_string());
    let entries = collect_catalog_entries(
        "deepseek/deepseek-v4-flash\ndeepseek/deepseek-v4-flash",
        &windows,
        &HashMap::new(),
        "",
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slug, "deepseek/deepseek-v4-flash");
    assert_eq!(entries[0].suffix_window, Some(200_000));
}

#[test]
fn migrate_model_list_with_suffixes_splits_slug_and_window() {
    let input = "deepseek-v4-flash[1M]\ndeepseek-v4-pro\nnvidia/...:free[200K]";
    let (clean_list, windows) =
        codex_plus_core::model_suffix::migrate_model_list_with_suffixes(input);
    assert_eq!(
        clean_list,
        "deepseek-v4-flash\ndeepseek-v4-pro\nnvidia/...:free"
    );
    assert_eq!(
        windows.get("deepseek-v4-flash"),
        Some(&"1000000".to_string())
    );
    assert_eq!(windows.get("deepseek-v4-pro"), None);
    assert_eq!(windows.get("nvidia/...:free"), Some(&"200000".to_string()));
}

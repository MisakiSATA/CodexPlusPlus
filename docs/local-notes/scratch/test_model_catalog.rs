use serde_json::{json, Value};

const GENERIC_TEMPLATE_JSON: &str = r#"{
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
}"#;

fn first_bundled_template_entry() -> Option<Value> {
    let catalog: Value = serde_json::from_str(GENERIC_TEMPLATE_JSON).ok()?;
    catalog.get("models")?.as_array()?.first().cloned()
}

fn main() {
    println!("Testing generic template loading...\n");

    if let Some(template) = first_bundled_template_entry() {
        println!("✓ Successfully loaded generic template");
        println!("  slug: {}", template.get("slug").and_then(Value::as_str).unwrap_or("N/A"));
        println!("  display_name: {}", template.get("display_name").and_then(Value::as_str).unwrap_or("N/A"));
        println!("  has base_instructions: {}", template.get("base_instructions").is_some());
        println!("  has shell_type: {}", template.get("shell_type").is_some());
        println!("  supported_in_api: {}", template.get("supported_in_api").and_then(Value::as_bool).unwrap_or(false));

        // 模拟为 Claude 模型生成目录条目
        let mut claude_model = template.clone();
        claude_model["slug"] = json!("claude-opus-4");
        claude_model["display_name"] = json!("Claude Opus 4");
        claude_model["description"] = json!("Claude Opus 4");
        claude_model["context_window"] = json!(200000);

        println!("\n✓ Generated catalog entry for Claude:");
        println!("{}", serde_json::to_string_pretty(&claude_model).unwrap());

    } else {
        println!("✗ Failed to load generic template");
    }
}

use serde_json::{Map, Value};

use super::{BackendSettings, RelayContextSelection, RelayProfile};

pub(super) fn normalize(settings: &mut BackendSettings) {
    let (common, legacy_common_context) = split_sections(&settings.relay_common_config_contents);
    let mut context = merge_sections(
        &settings.relay_context_config_contents,
        &legacy_common_context,
    );
    settings.relay_common_config_contents = common;

    for profile in &mut settings.relay_profiles {
        let (profile_config, legacy_profile_context) = split_sections(&profile.config_contents);
        profile.config_contents = profile_config;
        context = merge_sections(&context, &legacy_profile_context);
    }

    settings.relay_context_config_contents = normalize_text(context.clone());
    sync_profile_selections(settings, &context);
}

pub(super) fn persist_profile_fields(raw: &mut Map<String, Value>, profiles: &[RelayProfile]) {
    let Some(Value::Array(raw_profiles)) = raw.get_mut("relayProfiles") else {
        return;
    };
    for raw_profile in raw_profiles {
        let Some(raw_profile) = raw_profile.as_object_mut() else {
            continue;
        };
        let Some(profile) = raw_profile
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| profiles.iter().find(|profile| profile.id == id))
        else {
            continue;
        };
        if let Some(config_contents) = raw_profile.get("configContents").and_then(Value::as_str) {
            let (profile_config, _) = split_sections(config_contents);
            raw_profile.insert("configContents".to_string(), Value::String(profile_config));
        }
        raw_profile.insert(
            "contextSelection".to_string(),
            serde_json::to_value(&profile.context_selection)
                .unwrap_or_else(|_| Value::Object(Map::new())),
        );
        raw_profile.insert(
            "contextSelectionInitialized".to_string(),
            Value::Bool(profile.context_selection_initialized),
        );
    }
}

fn sync_profile_selections(settings: &mut BackendSettings, context: &str) {
    let Ok(entries) = crate::relay_config::list_context_entries_from_common_config(context) else {
        return;
    };
    let selection = RelayContextSelection {
        mcp_servers: entries
            .mcp_servers
            .into_iter()
            .map(|entry| entry.id)
            .collect(),
        skills: entries.skills.into_iter().map(|entry| entry.id).collect(),
        plugins: entries.plugins.into_iter().map(|entry| entry.id).collect(),
    };
    let should_sync = !selection.mcp_servers.is_empty()
        || !selection.skills.is_empty()
        || !selection.plugins.is_empty()
        || settings
            .relay_profiles
            .iter()
            .any(|profile| profile.context_selection_initialized);
    if should_sync {
        for profile in &mut settings.relay_profiles {
            profile.context_selection = selection.clone();
            profile.context_selection_initialized = true;
        }
    }
}

fn merge_sections(current: &str, incoming: &str) -> String {
    let current = current.trim();
    let incoming = incoming.trim();
    if current.is_empty() {
        return normalize_text(incoming.to_string());
    }
    if incoming.is_empty() {
        return normalize_text(current.to_string());
    }
    crate::relay_config::merge_common_config_into_config(incoming, current)
        .unwrap_or_else(|_| join_sections(&[current, incoming]))
}

fn split_sections(config: &str) -> (String, String) {
    let mut common = Vec::new();
    let mut context = Vec::new();
    let mut in_context_table = false;

    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_context_table = is_context_table_header(trimmed);
        }
        if in_context_table {
            context.push(line);
        } else {
            common.push(line);
        }
    }

    (
        normalize_text(common.join("\n")),
        normalize_text(context.join("\n")),
    )
}

fn is_context_table_header(header: &str) -> bool {
    ["mcp_servers", "skills", "plugins"]
        .into_iter()
        .any(|table| header == format!("[{table}]") || header.starts_with(&format!("[{table}.")))
}

fn join_sections(sections: &[&str]) -> String {
    let joined = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    normalize_text(joined)
}

fn normalize_text(contents: String) -> String {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

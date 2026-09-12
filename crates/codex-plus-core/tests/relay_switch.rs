use codex_plus_core::relay_switch::{
    acquire_relay_switch_lock, acquire_relay_switch_lock_async,
    acquire_relay_switch_lock_with_timeout, switch_relay_profile_in_home,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    LaunchMode, RelayMode, RelayProfile, RelaySessionProvider, SettingsStore,
};

#[test]
fn relay_switch_lock_wait_times_out_with_friendly_error_instead_of_hanging() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    let _holder = acquire_relay_switch_lock(&home).unwrap();

    let contender_home = home.clone();
    let started = std::time::Instant::now();
    let result = std::thread::spawn(move || {
        acquire_relay_switch_lock_with_timeout(
            &contender_home,
            std::time::Duration::from_millis(300),
        )
        .err()
        .map(|error| error.to_string())
    })
    .join()
    .unwrap();
    let error = result.expect("持锁期间的第二个获取必须在超时后报错，而不是无限等待");

    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(error.contains("超时"));
}

#[test]
fn relay_switch_file_lock_serializes_writers_for_the_same_codex_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    let first = acquire_relay_switch_lock(&home).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let contender_home = home.clone();
    let contender = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let _guard = acquire_relay_switch_lock(&contender_home).unwrap();
        acquired_tx.send(()).unwrap();
    });

    started_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(
        acquired_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    drop(first);
    acquired_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    contender.join().unwrap();
}

#[test]
fn relay_switch_async_wait_does_not_block_current_thread_executor() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    let runtime_home = home.clone();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();

    let runtime_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let first = acquire_relay_switch_lock(&runtime_home).unwrap();
            let (holder_started_tx, holder_started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let holder = tokio::spawn(async move {
                holder_started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                drop(first);
            });
            holder_started_rx.await.unwrap();

            let contender_home = runtime_home.clone();
            let contender = tokio::spawn(async move {
                let guard = acquire_relay_switch_lock_async(&contender_home)
                    .await
                    .unwrap();
                drop(guard);
            });
            tokio::task::yield_now().await;
            release_tx.send(()).unwrap();

            holder.await.unwrap();
            contender.await.unwrap();
        });
        finished_tx.send(()).unwrap();
    });

    finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("relay lock contention blocked the current-thread Tokio executor");
    runtime_thread.join().unwrap();
}

#[test]
fn switch_rejects_stale_previous_profile_after_waiting_for_lock() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://b.example/v1"
"#,
    )
    .unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-b"}"#).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let current = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            pure_profile("b", "https://b.example/v1", "sk-b"),
            pure_profile("c", "https://c.example/v1", "sk-c"),
        ],
        ..BackendSettings::default()
    };
    store.save(&current).unwrap();
    let stale_request = BackendSettings {
        active_relay_id: "c".to_string(),
        relay_profiles: current.relay_profiles.clone(),
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &home, stale_request, "a")
        .expect_err("stale request must not overwrite the current provider");

    assert!(error.to_string().contains("切换请求已过期"));
    let stored = store.load().unwrap();
    assert_eq!(stored.active_relay_id, "b");
    assert_eq!(
        stored
            .relay_profiles
            .iter()
            .find(|profile| profile.id == "a")
            .unwrap()
            .base_url,
        "https://a.example/v1"
    );
    assert!(
        std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("https://b.example/v1")
    );
}

#[test]
fn switch_rolls_back_active_settings_when_live_write_fails() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![pure_profile("a", "https://a.example/v1", "sk-a")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    std::fs::create_dir(temp.path().join("codex")).unwrap();
    std::fs::write(
        temp.path().join("codex").join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-a"}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("codex").join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#,
    )
    .unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            RelayProfile {
                id: "b".to_string(),
                name: "B".to_string(),
                relay_mode: RelayMode::PureApi,
                config_contents: "model_provider = \"custom\"\n".to_string(),
                auth_contents: "{bad json".to_string(),
                ..RelayProfile::default()
            },
        ],
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &temp.path().join("codex"), next, "a")
        .expect_err("invalid auth should fail switch");

    assert!(error.to_string().contains("auth.json"));
    assert_eq!(store.load().unwrap().active_relay_id, "a");
    assert!(
        std::fs::read_to_string(temp.path().join("codex").join("config.toml"))
            .unwrap()
            .contains("https://a.example/v1")
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("codex").join("auth.json")).unwrap(),
        r#"{"OPENAI_API_KEY":"sk-a"}"#
    );
}

#[test]
fn switch_rolls_back_live_files_when_post_write_status_check_fails() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let original_auth = r#"{"OPENAI_API_KEY":"sk-a"}"#;
    let original_config = r#"model_provider = "custom"

[hooks.state."plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
trusted_hash = "live-a-hash"

[hooks.state."plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
trusted_hash = "live-b-hash"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#;
    std::fs::write(home.join("auth.json"), original_auth).unwrap();
    std::fs::write(home.join("config.toml"), original_config).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![pure_profile("a", "https://a.example/v1", "sk-a")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let persisted_original = store.load().unwrap();
    let original_settings_bytes = std::fs::read(temp.path().join("settings.json")).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            RelayProfile {
                id: "b".to_string(),
                name: "B".to_string(),
                relay_mode: RelayMode::PureApi,
                config_contents: r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://b.example/v1"
"#
                .to_string(),
                auth_contents: "{}".to_string(),
                ..RelayProfile::default()
            },
        ],
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &home, next, "a")
        .expect_err("missing api key should fail post-write status check");

    assert!(
        error
            .to_string()
            .contains("纯 API 配置写入后未检测到完整 custom provider")
    );
    assert_eq!(store.load().unwrap(), persisted_original);
    assert_eq!(
        std::fs::read(temp.path().join("settings.json")).unwrap(),
        original_settings_bytes
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        original_auth
    );
}

#[test]
fn switch_rolls_back_managed_model_catalog_when_later_live_write_fails() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    let catalog_path = home.join("model-catalogs/a.json");
    std::fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
    let original_catalog = br#"{"models":[{"slug":"old-model"}]}"#;
    std::fs::write(&catalog_path, original_catalog).unwrap();
    let original_config = r#"model = "old-model"
model_provider = "custom"
model_catalog_json = "model-catalogs/a.json"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#;
    let original_auth = r#"{"OPENAI_API_KEY":"sk-a"}"#;
    std::fs::write(home.join("config.toml"), original_config).unwrap();
    std::fs::write(home.join("auth.json"), original_auth).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut original_profile = pure_profile("a", "https://a.example/v1", "sk-a");
    original_profile.model = "old-model".to_string();
    original_profile.model_list = "old-model".to_string();
    original_profile.config_contents = original_config.to_string();
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![original_profile.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let stored_before = store.load().unwrap();
    let mut failed_profile = original_profile;
    failed_profile.model = "grok-4".to_string();
    failed_profile.model_list = "grok-4\nclaude-sonnet-4".to_string();
    failed_profile.model_windows = r#"{"grok-4":"1000000"}"#.to_string();
    failed_profile.auth_contents = "{invalid auth json".to_string();
    let next = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![failed_profile],
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &home, next, "a")
        .expect_err("invalid auth must fail after catalog generation");

    assert!(error.to_string().contains("auth.json"));
    assert_eq!(store.load().unwrap(), stored_before);
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        original_auth
    );
    assert_eq!(std::fs::read(&catalog_path).unwrap(), original_catalog);
}

#[test]
fn switch_backfills_previous_profile_from_live_before_selecting_target() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model = "edited-live-model"
model_provider = "manual_a"
model_context_window = 1000000
model_auto_compact_token_limit = 900000

[hooks.state."plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
trusted_hash = "live-a-hash"

[hooks.state."plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
trusted_hash = "live-b-hash"

[model_providers.manual_a]
name = "manual_a"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://edited-a.example/v1"
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-edited-a"}"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            pure_profile("b", "https://b.example/v1", "sk-b"),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: original.relay_profiles.clone(),
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();

    let stored = store.load().unwrap();
    let previous = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "a")
        .unwrap();
    assert!(previous.config_contents.contains("edited-live-model"));
    assert!(previous.config_contents.contains("manual_a"));
    assert_eq!(previous.context_window, "1000000");
    assert_eq!(previous.auto_compact_limit, "900000");
    assert_eq!(stored.active_relay_id, "b");
    assert_eq!(stored.launch_mode, LaunchMode::Patch);
    let live: toml::Value = std::fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        live["hooks"]["state"]["plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
            ["trusted_hash"]
            .as_str(),
        Some("live-a-hash")
    );
    assert_eq!(
        live["hooks"]["state"]["plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
            ["trusted_hash"]
            .as_str(),
        Some("live-b-hash")
    );
}

#[test]
fn switch_to_aggregate_relay_allows_empty_config_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    let aggregate = RelayProfile {
        id: "agg".to_string(),
        name: "聚合供应商 1".to_string(),
        relay_mode: RelayMode::Aggregate,
        config_contents: String::new(),
        auth_contents: String::new(),
        ..RelayProfile::default()
    };
    let original = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![api.clone(), aggregate.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "agg".to_string(),
        relay_profiles: vec![api, aggregate],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "聚合供应商 1".to_string(),
            session_provider: RelaySessionProvider::Custom,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: "api".to_string(),
                weight: 1,
            }],
            routes: Vec::new(),
        }],
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    };

    let result = switch_relay_profile_in_home(&store, &home, next, "api").unwrap();
    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();

    assert!(result.configured);
    assert_eq!(store.load().unwrap().active_relay_id, "agg");
    assert!(live.contains(r#"base_url = "http://127.0.0.1:57321/v1""#));
}

#[test]
fn switch_returns_normalized_previous_official_profile_after_backfill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model = "gpt-5.5"
model_reasoning_effort = "high"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://third-party.example/v1"

[features]
goals = true
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-third-party"}"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let official = RelayProfile {
        id: "official".to_string(),
        name: "官方".to_string(),
        relay_mode: RelayMode::Official,
        official_mix_api_key: false,
        hide_official_usage_alert: false,
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    let pure = pure_profile("api", "https://third-party.example/v1", "sk-third-party");
    let original = BackendSettings {
        active_relay_id: "official".to_string(),
        relay_profiles: vec![official.clone(), pure.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![official, pure],
        ..BackendSettings::default()
    };

    let result = switch_relay_profile_in_home(&store, &home, next, "official").unwrap();
    let returned = result
        .settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "official")
        .unwrap();

    assert_eq!(returned.relay_mode, RelayMode::Official);
    assert!(!returned.official_mix_api_key);
    assert!(returned.config_contents.is_empty());
    assert!(returned.api_key.is_empty());
}

#[test]
fn switch_captures_safe_app_state_before_writing_provider_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join(".codex-global-state.json"),
        serde_json::json!({
            "electron-saved-workspace-roots": ["C:/work/app"],
            "prompt-history": ["do-not-copy"],
            "electron-persisted-atom-state": {
                "default-service-tier": "priority",
                "provider-token-cache": "do-not-copy"
            }
        })
        .to_string(),
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            pure_profile("b", "https://b.example/v1", "sk-b"),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: original.relay_profiles.clone(),
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            home.join("backups_state")
                .join("app-state-sync")
                .join("latest-safe-state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    // 路径按当前平台风格规范化：Windows 归一化为反斜杠，Unix 原样保留。
    let expected_root = if cfg!(windows) {
        "C:\\work\\app"
    } else {
        "C:/work/app"
    };
    assert_eq!(
        snapshot["state"]["electron-saved-workspace-roots"],
        serde_json::json!([expected_root])
    );
    assert_eq!(
        snapshot["state"]["electron-persisted-atom-state"]["default-service-tier"],
        "priority"
    );
    assert!(snapshot["state"].get("prompt-history").is_none());
    assert!(
        snapshot["state"]["electron-persisted-atom-state"]
            .get("provider-token-cache")
            .is_none()
    );
}

#[test]
fn consecutive_channel_switches_follow_each_channels_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let gpt = channel_profile("gpt", "https://gpt.example/v1", "sk-gpt", "gpt-5.5");
    let claude = channel_profile(
        "claude",
        "https://claude.example/v1",
        "sk-claude",
        "claude-opus-5",
    );
    let mut grok = channel_profile("grok", "https://grok.example/v1", "sk-grok", "grok-4.5");
    grok.config_contents = format!(
        "model_context_window = 2000000\nmodel_auto_compact_token_limit = 1800000\n{}",
        grok.config_contents
    );
    let official = RelayProfile {
        id: "official".to_string(),
        name: "官方".to_string(),
        relay_mode: RelayMode::Official,
        official_mix_api_key: false,
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    std::fs::write(home.join("config.toml"), &gpt.config_contents).unwrap();
    std::fs::write(home.join("auth.json"), &gpt.auth_contents).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let profiles = vec![gpt, claude, grok, official];
    let original = BackendSettings {
        active_relay_id: "gpt".to_string(),
        relay_profiles: profiles.clone(),
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();

    // gpt → claude：模型与 base_url 都必须跟着 Claude 渠道走
    let to_claude = BackendSettings {
        active_relay_id: "claude".to_string(),
        relay_profiles: profiles.clone(),
        ..BackendSettings::default()
    };
    switch_relay_profile_in_home(&store, &home, to_claude, "gpt").unwrap();
    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(live.contains(r#"model = "claude-opus-5""#));
    assert!(live.contains("https://claude.example/v1"));
    assert!(!live.contains("gpt-5.5"));

    // claude → grok
    let stored = store.load().unwrap();
    let to_grok = BackendSettings {
        active_relay_id: "grok".to_string(),
        relay_profiles: stored.relay_profiles.clone(),
        ..stored
    };
    switch_relay_profile_in_home(&store, &home, to_grok, "claude").unwrap();
    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(live.contains(r#"model = "grok-4.5""#));
    assert!(live.contains("https://grok.example/v1"));
    assert!(!live.contains("claude-opus-5"));

    // grok → 官方：第三方渠道的模型与上下文残留必须清掉，避免官方 OpenAI 渠道拿到 grok 模型
    let stored = store.load().unwrap();
    let to_official = BackendSettings {
        active_relay_id: "official".to_string(),
        relay_profiles: stored.relay_profiles.clone(),
        ..stored
    };
    switch_relay_profile_in_home(&store, &home, to_official, "grok").unwrap();
    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();
    // 根级 model_provider 必须清掉；[model_providers.custom] 表按上游语义保留但不得带密钥。
    assert!(!live.contains("model_provider ="));
    assert!(!live.contains("sk-grok"));
    assert!(!live.contains("grok-4.5"));
    assert!(!live.contains("model_context_window"));
    assert!(!live.contains("model_auto_compact_token_limit"));
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth["auth_mode"], "chatgpt");
}

fn channel_profile(id: &str, base_url: &str, key: &str, model: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::PureApi,
        model: model.to_string(),
        config_contents: format!(
            r#"model = "{model}"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "{base_url}"
"#
        ),
        auth_contents: format!(r#"{{"OPENAI_API_KEY":"{key}"}}"#),
        ..RelayProfile::default()
    }
}

fn pure_profile(id: &str, base_url: &str, key: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::PureApi,
        config_contents: format!(
            r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "{base_url}"
"#
        ),
        auth_contents: format!(r#"{{"OPENAI_API_KEY":"{key}"}}"#),
        ..RelayProfile::default()
    }
}

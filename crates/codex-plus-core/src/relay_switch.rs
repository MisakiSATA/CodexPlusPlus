use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;
use fs2::FileExt;

use crate::relay_config::{
    backfill_relay_profile_from_home_with_common, relay_config_status_from_home,
};
use crate::settings::{BackendSettings, RelayMode, SettingsStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelaySwitchResult {
    pub settings: BackendSettings,
    pub configured: bool,
    pub backup_path: Option<String>,
}

pub struct RelaySwitchLockGuard {
    file: std::fs::File,
}

impl Drop for RelaySwitchLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// 默认锁等待上限。切换/启动的配置写入阶段通常几秒内完成；等不到锁时必须报错返回，
/// 绝不能把调用方（尤其是 Manager 的命令线程）无限挂起，否则界面会整体卡死。
pub const RELAY_SWITCH_LOCK_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const RELAY_SWITCH_LOCK_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

pub fn acquire_relay_switch_lock(home: &Path) -> anyhow::Result<RelaySwitchLockGuard> {
    acquire_relay_switch_lock_with_timeout(home, RELAY_SWITCH_LOCK_WAIT_TIMEOUT)
}

pub fn acquire_relay_switch_lock_with_timeout(
    home: &Path,
    timeout: std::time::Duration,
) -> anyhow::Result<RelaySwitchLockGuard> {
    let lock_dir = home.join("tmp");
    std::fs::create_dir_all(&lock_dir)
        .with_context(|| format!("创建供应商切换锁目录失败：{}", lock_dir.to_string_lossy()))?;
    let lock_path = lock_dir.join("codex-plus-relay-switch.lock");
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("打开供应商切换锁失败：{}", lock_path.to_string_lossy()))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(RelaySwitchLockGuard { file }),
            Err(error) if is_lock_contention(&error) => {
                if std::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "等待供应商切换锁超时（{} 秒）：另一个供应商切换、导入或 Codex 启动可能正在进行，请稍后重试。锁文件：{}",
                        timeout.as_secs().max(1),
                        lock_path.to_string_lossy()
                    );
                }
                std::thread::sleep(RELAY_SWITCH_LOCK_RETRY_INTERVAL);
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "获取供应商切换锁失败：{}",
                    lock_path.to_string_lossy()
                )));
            }
        }
    }
}

fn is_lock_contention(error: &std::io::Error) -> bool {
    let contended = fs2::lock_contended_error();
    error.kind() == contended.kind() && error.raw_os_error() == contended.raw_os_error()
}

pub async fn acquire_relay_switch_lock_async(home: &Path) -> anyhow::Result<RelaySwitchLockGuard> {
    let home = home.to_path_buf();
    tokio::task::spawn_blocking(move || acquire_relay_switch_lock(&home))
        .await
        .context("等待供应商切换锁任务失败")?
}

pub fn switch_relay_profile_in_home(
    store: &SettingsStore,
    home: &Path,
    next_settings: BackendSettings,
    previous_active_relay_id: &str,
) -> anyhow::Result<RelaySwitchResult> {
    let relay_switch_lock = acquire_relay_switch_lock(home)?;
    switch_relay_profile_in_home_with_lock(
        store,
        home,
        next_settings,
        previous_active_relay_id,
        &relay_switch_lock,
    )
}

pub fn switch_relay_profile_in_home_with_lock(
    store: &SettingsStore,
    home: &Path,
    next_settings: BackendSettings,
    previous_active_relay_id: &str,
    _relay_switch_lock: &RelaySwitchLockGuard,
) -> anyhow::Result<RelaySwitchResult> {
    let mut selected_settings = next_settings;
    if !selected_settings.relay_profiles_enabled {
        anyhow::bail!("供应商配置总开关已关闭，未写入 config.toml / auth.json。");
    }
    crate::codex_app_state::capture_app_state_snapshot_nonfatal(home, "relay_switch.before");

    let original_settings = store.load().context("读取当前供应商设置失败")?;
    let previous_active_relay_id = previous_active_relay_id.trim();
    if !previous_active_relay_id.is_empty()
        && original_settings.active_relay_id != previous_active_relay_id
    {
        anyhow::bail!(
            "供应商切换请求已过期：当前供应商已从「{}」变为「{}」，请刷新后重试。",
            previous_active_relay_id,
            original_settings.active_relay_id
        );
    }
    let selected_catalog_path = crate::relay_config::managed_model_catalog_path(
        home,
        &selected_settings.active_relay_profile().id,
    );
    let live_snapshot = LiveFilesSnapshot::capture(home, selected_catalog_path)
        .context("读取当前 Codex 实时配置失败")?;
    if !previous_active_relay_id.trim().is_empty()
        && previous_active_relay_id != selected_settings.active_relay_id
    {
        backfill_profile_before_switch(home, &mut selected_settings, previous_active_relay_id)?;
    }

    let switch_result = (|| {
        store
            .save(&selected_settings)
            .context("保存供应商设置失败")?;
        let selected_settings = store.load().context("读取供应商设置失败")?;
        apply_selected_relay_profile(home, &selected_settings)
    })();

    match switch_result {
        Ok(result) => {
            crate::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
                home,
                "relay_switch.after",
            );
            Ok(result)
        }
        Err(error) => {
            let settings_restore_error = store.save(&original_settings).err();
            let live_restore_error = live_snapshot.restore(home).err();
            if settings_restore_error.is_some() || live_restore_error.is_some() {
                anyhow::bail!(
                    "切换供应商失败：{error}；同时回滚配置失败：settings.json={}，Codex 实时文件={}",
                    settings_restore_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "ok".to_string()),
                    live_restore_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "ok".to_string())
                );
            }
            Err(error)
        }
    }
}

#[derive(Debug, Clone)]
struct LiveFilesSnapshot {
    config: Option<Vec<u8>>,
    auth: Option<Vec<u8>>,
    managed_catalog_path: std::path::PathBuf,
    managed_catalog: Option<Vec<u8>>,
}

impl LiveFilesSnapshot {
    fn capture(home: &Path, managed_catalog_path: std::path::PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            config: read_optional_bytes(&home.join("config.toml"))?,
            auth: read_optional_bytes(&home.join("auth.json"))?,
            managed_catalog: read_optional_bytes(&managed_catalog_path)?,
            managed_catalog_path,
        })
    }

    fn restore(&self, home: &Path) -> anyhow::Result<()> {
        let mut errors = Vec::new();
        if let Err(error) =
            restore_optional_file(&self.managed_catalog_path, self.managed_catalog.as_deref())
        {
            errors.push(format!("恢复模型 catalog 失败：{error:#}"));
        }
        if let Err(error) = std::fs::create_dir_all(home) {
            errors.push(format!("恢复 Codex 配置目录失败：{error:#}"));
        }
        if let Err(error) = restore_optional_file(&home.join("config.toml"), self.config.as_deref())
        {
            errors.push(format!("恢复 config.toml 失败：{error:#}"));
        }
        if let Err(error) = restore_optional_file(&home.join("auth.json"), self.auth.as_deref()) {
            errors.push(format!("恢复 auth.json 失败：{error:#}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("；"))
        }
    }
}

fn read_optional_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore_optional_file(path: &Path, contents: Option<&[u8]>) -> anyhow::Result<()> {
    match contents {
        Some(contents) => crate::settings::atomic_write(path, contents).map_err(Into::into),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

fn backfill_profile_before_switch(
    home: &Path,
    settings: &mut BackendSettings,
    previous_active_relay_id: &str,
) -> anyhow::Result<()> {
    let profile = settings
        .relay_profiles
        .iter_mut()
        .find(|profile| profile.id == previous_active_relay_id)
        .with_context(|| "当前供应商已不在配置列表中，已停止切换以避免覆盖用户改动。")?;
    backfill_relay_profile_from_home_with_common(
        home,
        profile,
        &mut settings.relay_context_config_contents,
    )
    .with_context(|| "回填当前供应商配置失败")
}

fn apply_selected_relay_profile(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<RelaySwitchResult> {
    let relay = settings.active_relay_profile();
    let common_config = relay_combined_common_config(settings);
    let result = if relay.relay_mode == RelayMode::Official && !relay.official_mix_api_key {
        let auth_contents =
            (!relay.auth_contents.trim().is_empty()).then_some(relay.auth_contents.as_str());
        crate::relay_config::clear_relay_config_for_official_switch(home, auth_contents)?
    } else {
        validate_switch_profile_files(&relay)?;
        crate::relay_config::apply_relay_profile_to_home_with_switch_rules(
            home,
            &relay,
            &common_config,
        )?
    };
    let status = relay_config_status_from_home(home);
    if relay.relay_mode == RelayMode::PureApi && !status.configured {
        anyhow::bail!(
            "纯 API 配置写入后未检测到完整 custom provider，请检查 config.toml 和供应商 API Key。"
        );
    }
    Ok(RelaySwitchResult {
        settings: settings.clone(),
        configured: status.configured,
        backup_path: result.backup_path,
    })
}

fn validate_switch_profile_files(profile: &crate::settings::RelayProfile) -> anyhow::Result<()> {
    if profile.relay_mode != RelayMode::Aggregate && profile.config_contents.trim().is_empty() {
        anyhow::bail!(
            "供应商「{}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。",
            if profile.name.trim().is_empty() {
                profile.id.as_str()
            } else {
                profile.name.as_str()
            }
        );
    }
    if profile.relay_mode == RelayMode::Official
        && serde_json::from_str::<serde_json::Value>(&profile.auth_contents)
            .ok()
            .and_then(|value| {
                value
                    .get("OPENAI_API_KEY")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .map(str::is_empty)
            })
            == Some(false)
    {
        anyhow::bail!(
            "官方混合 API 不应在 auth.json 中保存 OPENAI_API_KEY。请清理此供应商的 auth.json 后再切换。"
        );
    }
    Ok(())
}

fn relay_combined_common_config(settings: &BackendSettings) -> String {
    let sections = [
        settings.relay_common_config_contents.trim(),
        settings.relay_context_config_contents.trim(),
    ]
    .into_iter()
    .filter(|section| !section.is_empty())
    .collect::<Vec<_>>();
    if sections.is_empty() {
        String::new()
    } else {
        crate::relay_config::normalize_config_text(&format!("{}\n", sections.join("\n\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_snapshot_restore_attempts_every_file_and_aggregates_errors() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let catalog_path = home.join("model-catalogs/profile.json");
        std::fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
        std::fs::write(home.join("config.toml"), "model = \"original\"\n").unwrap();
        std::fs::write(home.join("auth.json"), "{\"token\":\"original\"}\n").unwrap();
        std::fs::write(&catalog_path, "{\"models\":[]}").unwrap();
        let snapshot = LiveFilesSnapshot::capture(&home, catalog_path.clone()).unwrap();

        std::fs::write(home.join("config.toml"), "model = \"changed\"\n").unwrap();
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::create_dir(home.join("auth.json")).unwrap();
        std::fs::remove_file(&catalog_path).unwrap();
        std::fs::create_dir(&catalog_path).unwrap();

        let error = snapshot
            .restore(&home)
            .expect_err("directory targets must make catalog and auth restoration fail");
        let message = error.to_string();

        assert!(message.contains("模型 catalog"), "{message}");
        assert!(message.contains("auth.json"), "{message}");
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            "model = \"original\"\n"
        );
    }
}

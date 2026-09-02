use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::settings::{BackendSettings, SettingsStore, normalize_codex_extra_args};
use crate::status::{LaunchStatus, StatusStore};

#[cfg(windows)]
const POST_LAUNCH_COMPUTER_USE_GUARD_SECONDS: &[u64] = &[0, 5, 15, 30, 60, 120, 180, 240, 300];
#[cfg_attr(not(windows), allow(dead_code))]
const POST_LAUNCH_COMPUTER_USE_GUARD_STABLE_ATTEMPTS: usize = 3;
static PET_OVERLAY_SYNC_FAILED: AtomicBool = AtomicBool::new(false);
static PET_CURSOR_DRIVER_FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexLaunch {
    Process {
        command: Vec<String>,
        wait_strategy: ProcessWaitStrategy,
        macos_cleanup_policy: Option<MacosCleanupPolicy>,
    },
    PackagedActivation {
        app_user_model_id: String,
        arguments: String,
        process_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessWaitStrategy {
    TrackedChild,
    ExternalWaitCommand,
}

#[cfg(target_os = "linux")]
const PROCESS_EXIT_EMPTY_OBSERVATIONS: u32 = 15;
#[cfg(not(target_os = "linux"))]
const PROCESS_EXIT_EMPTY_OBSERVATIONS: u32 = 3;

#[derive(Debug, Default)]
struct ProcessExitObservation {
    empty_streak: u32,
}

impl ProcessExitObservation {
    fn new() -> Self {
        Self::default()
    }

    fn observe(&mut self, has_codex_process: bool) -> bool {
        if has_codex_process {
            self.empty_streak = 0;
            return false;
        }
        self.empty_streak = self.empty_streak.saturating_add(1);
        self.empty_streak >= PROCESS_EXIT_EMPTY_OBSERVATIONS
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacosCleanupPolicy {
    QuitIfNotPreviouslyRunning,
    SkipQuitBecauseAlreadyRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsProcessControlStrategy {
    NativeWindowsApi,
}

#[cfg(windows)]
pub fn windows_process_control_strategy() -> WindowsProcessControlStrategy {
    WindowsProcessControlStrategy::NativeWindowsApi
}

impl CodexLaunch {
    pub fn process_id(&self) -> Option<u32> {
        match self {
            Self::PackagedActivation { process_id, .. } => *process_id,
            Self::Process { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub app_dir: Option<PathBuf>,
    pub debug_port: u16,
    pub helper_port: u16,
    pub status_store: StatusStore,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            app_dir: None,
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::default(),
        }
    }
}

#[derive(Clone)]
pub struct LaunchHandle {
    pub debug_port: u16,
    pub helper_port: u16,
    pub app_dir: PathBuf,
    pub launch: CodexLaunch,
    pub status_store: StatusStore,
    /// Timestamp written with this launch's active status.  It acts as a lightweight
    /// ownership token so an older handle cannot overwrite a newer launch using the
    /// same ports and app path.
    started_at_ms: u64,
    helper_started: bool,
    hooks: Arc<dyn LaunchHooks>,
}

impl std::fmt::Debug for LaunchHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchHandle")
            .field("debug_port", &self.debug_port)
            .field("helper_port", &self.helper_port)
            .field("app_dir", &self.app_dir)
            .field("launch", &self.launch)
            .field("status_store", &self.status_store)
            .finish_non_exhaustive()
    }
}

impl LaunchHandle {
    pub async fn wait_for_codex_exit(&self) -> anyhow::Result<()> {
        let result = self.hooks.wait_for_codex_exit(&self.launch).await;
        self.persist_terminal_status(&result);
        if self.helper_started {
            self.hooks.shutdown_helper(self.helper_port).await;
        }
        result
    }

    /// Persist the terminal state only when this handle still owns the active launch record.
    ///
    /// A launcher can outlive the manager window, and a later launch may replace the status
    /// file before an older handle observes its process exit.  In that case blindly writing
    /// `stopped` would clobber the newer launch state, so we require the record to still match
    /// this handle and to be in one of the active states.
    fn persist_terminal_status(&self, result: &anyhow::Result<()>) {
        let app_dir = self.app_dir.to_string_lossy().to_string();
        let update_result = self.status_store.update_latest_if(|current| {
            let mut current = current?;
            if current.started_at_ms != self.started_at_ms
                || current.debug_port != Some(self.debug_port)
                || current.helper_port != Some(self.helper_port)
                || current.codex_app.as_deref() != Some(app_dir.as_str())
                || !matches!(current.status.as_str(), "running" | "running_degraded")
            {
                return None;
            }

            if result.is_ok() {
                current.status = "stopped".to_string();
                current.message = "Codex process exited".to_string();
            } else {
                current.status = "crashed".to_string();
                current.message = format!(
                    "Codex process wait failed: {}",
                    result.as_ref().unwrap_err()
                );
            }
            Some(current)
        });
        if let Err(error) = update_result {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.terminal_status_persist_failed",
                serde_json::json!({
                    "message": error.to_string(),
                    "debug_port": self.debug_port,
                    "helper_port": self.helper_port
                }),
            );
        }
    }
}

/// 启动阶段注入重试的总时长上限。页面 30 秒未就绪后注入重试是延长的就绪探测，
/// 但它可能在持有供应商切换锁的页面未就绪路径上运行，必须限时结束。
pub const STARTUP_INJECTION_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(90);

#[async_trait(?Send)]
pub trait LaunchHooks: Send + Sync {
    fn resolve_codex_home(&self) -> PathBuf {
        crate::relay_config::default_codex_home_dir()
    }
    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf>;
    fn select_debug_port(&self, requested: u16) -> u16;
    fn select_helper_port(&self, requested: u16) -> u16;
    async fn load_settings(&self) -> anyhow::Result<BackendSettings>;
    async fn run_provider_sync(&self, codex_home: &Path) -> anyhow::Result<()>;
    async fn apply_active_relay_profile(
        &self,
        _settings: &BackendSettings,
        _codex_home: &Path,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn ensure_computer_use_config(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
        Ok(())
    }
    async fn ensure_plugin_marketplace_config(
        &self,
        _settings: &BackendSettings,
        _relay_switch_lock: &crate::relay_switch::RelaySwitchLockGuard,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn sync_dream_skin_base_theme(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        crate::dream_skin::sync_default_dream_skin_base_theme(
            settings.enhancements_enabled
                && settings.codex_app_dream_skin_enabled
                && !settings.codex_app_dream_skin_paused,
            &settings.codex_app_dream_skin_theme_config,
        )
    }
    fn sanitize_historical_model_suffixes(
        &self,
        codex_home: &Path,
    ) -> anyhow::Result<crate::codex_sqlite::SanitizeModelSuffixResult> {
        crate::codex_sqlite::sanitize_historical_model_suffixes(codex_home)
    }
    async fn sanitize_local_storage_model_suffixes(&self, debug_port: u16) {
        crate::codex_local_storage::sanitize_local_storage_model_suffixes_nonfatal(debug_port)
            .await;
    }
    async fn wait_for_codex_config_load(&self, debug_port: u16) -> anyhow::Result<()> {
        wait_for_codex_page(debug_port).await
    }
    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()>;
    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        settings: &BackendSettings,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch>;
    async fn bridge_context(
        &self,
        _debug_port: u16,
        _app_dir: &Path,
    ) -> anyhow::Result<Option<crate::routes::BridgeContext>> {
        Ok(None)
    }
    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()>;
    async fn inject_bridge(
        &self,
        debug_port: u16,
        helper_port: u16,
        _ctx: crate::routes::BridgeContext,
    ) -> anyhow::Result<()> {
        self.inject(debug_port, helper_port).await
    }
    async fn ensure_injection(&self, debug_port: u16, helper_port: u16, app_dir: &Path) -> bool {
        // CDP 端口一直不可达时必须限时放弃：这段循环可能在页面未就绪路径上
        // 持有供应商切换锁运行，无界重试会把 Manager 的切换/保存拖住几十分钟。
        let deadline = std::time::Instant::now() + STARTUP_INJECTION_RETRY_WINDOW;
        let mut attempt = 0_u32;
        loop {
            attempt += 1;
            let result = match self.bridge_context(debug_port, app_dir).await {
                Ok(Some(ctx)) => self.inject_bridge(debug_port, helper_port, ctx).await,
                Ok(None) => self.inject(debug_port, helper_port).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => return true,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.ensure_injection_retry_failed",
                        serde_json::json!({
                            "debug_port": debug_port,
                            "helper_port": helper_port,
                            "attempt": attempt,
                            "message": error.to_string()
                        }),
                    );
                    if std::time::Instant::now() >= deadline {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.ensure_injection_gave_up",
                            serde_json::json!({
                                "debug_port": debug_port,
                                "helper_port": helper_port,
                                "attempts": attempt,
                                "window_secs": STARTUP_INJECTION_RETRY_WINDOW.as_secs()
                            }),
                        );
                        return false;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }
    async fn start_bridge_watchdog(
        &self,
        _debug_port: u16,
        _helper_port: u16,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn start_computer_use_guard_watchdog(
        &self,
        _settings: &BackendSettings,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write_status(&self, status: &str);
    /// Return whether the just-launched Codex process is known to still be alive.
    /// Implementations should be conservative and return `true` when process state cannot
    /// be queried reliably, so a transient inspection failure never kills a healthy app.
    async fn codex_process_is_alive(&self, _launch: &CodexLaunch) -> bool {
        true
    }
    async fn wait_for_codex_exit(&self, launch: &CodexLaunch) -> anyhow::Result<()>;
    async fn shutdown_helper(&self, helper_port: u16);
    async fn terminate_codex(&self, launch: &CodexLaunch) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct DefaultLaunchHooks {
    child: Mutex<Option<Child>>,
    helper: Mutex<Option<HelperRuntime>>,
    bridge_watchdog: Mutex<Option<BridgeWatchdogRuntime>>,
    computer_use_guard_watchdog: Mutex<Option<ComputerUseGuardWatchdogRuntime>>,
    computer_use_guard_artifacts: Mutex<Option<crate::computer_use_guard::GuardArtifacts>>,
    #[cfg(windows)]
    launched_process_handle: Mutex<Option<WindowsProcessHandle>>,
    #[cfg(windows)]
    packaged_process: Mutex<Option<WindowsPackagedProcess>>,
    #[cfg(target_os = "linux")]
    launched_process_group: Mutex<Option<u32>>,
}

struct HelperRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    active_connections: Arc<AtomicUsize>,
}

struct ActiveHelperConnection(Arc<AtomicUsize>);

impl ActiveHelperConnection {
    fn new(active_connections: Arc<AtomicUsize>) -> Self {
        active_connections.fetch_add(1, Ordering::AcqRel);
        Self(active_connections)
    }
}

impl Drop for ActiveHelperConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct BridgeWatchdogRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

struct ComputerUseGuardWatchdogRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

pub async fn launch_and_inject(options: LaunchOptions) -> anyhow::Result<LaunchHandle> {
    launch_and_inject_with_hooks(options, DefaultLaunchHooks::shared()).await
}

pub async fn launch_and_inject_with_hooks<H>(
    options: LaunchOptions,
    hooks: H,
) -> anyhow::Result<LaunchHandle>
where
    H: IntoLaunchHooks,
{
    let hooks = hooks.into_launch_hooks();
    let debug_port = hooks.select_debug_port(options.debug_port);
    let mut helper_port = hooks.select_helper_port(options.helper_port);
    let home = hooks.resolve_codex_home();
    let mut relay_switch_lock =
        Some(crate::relay_switch::acquire_relay_switch_lock_async(&home).await?);
    let settings = hooks.load_settings().await?;
    let app_dir = hooks.resolve_app_dir(options.app_dir.as_deref(), &settings)?;
    let status_store = options.status_store.clone();
    let mut helper_started = false;
    let mut launched = None;
    let mut launch_started_at_ms = None;

    let result: anyhow::Result<LaunchHandle> = async {
        if should_apply_active_relay_profile_at_launch(&settings) {
            hooks.apply_active_relay_profile(&settings, &home).await?;
        }
        if settings.provider_sync_enabled {
            crate::codex_app_state::capture_app_state_snapshot_nonfatal(&home, "launcher.before");
            hooks.run_provider_sync(&home).await?;
            crate::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
                &home,
                "launcher.after_provider_sync",
            );
        }
        hooks.sync_dream_skin_base_theme(&settings)?;
        let active_relay_switch_lock = relay_switch_lock
            .as_ref()
            .context("供应商切换锁在启动配置完成前被提前释放")?;
        if let Err(error) = hooks
            .ensure_plugin_marketplace_config(&settings, active_relay_switch_lock)
            .await
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.plugin_marketplace_config_failed_nonfatal",
                serde_json::json!({
                    "message": error.to_string()
                }),
            );
        }
        if settings.computer_use_guard_enabled {
            hooks.ensure_computer_use_config(&settings).await?;
        }
        match hooks.sanitize_historical_model_suffixes(&home) {
            Ok(result) if result.updated > 0 => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.sanitize_historical_model_suffixes",
                    serde_json::json!({
                        "scanned": result.scanned,
                        "updated": result.updated
                    }),
                );
            }
            Ok(_) => {}
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.sanitize_historical_model_suffixes_failed",
                    serde_json::json!({
                        "error": error.to_string()
                    }),
                );
            }
        }
        let protocol_proxy_enabled = relay_protocol_proxy_enabled(&settings);
        if protocol_proxy_enabled {
            helper_port = crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT;
        }
        if settings.enhancements_enabled || protocol_proxy_enabled {
            hooks.start_helper(helper_port).await?;
            helper_started = true;
        }

        let launch = hooks
            .launch_codex(&app_dir, debug_port, &settings, &settings.codex_extra_args)
            .await?;
        launched = Some(launch.clone());
        if settings.computer_use_guard_enabled {
            hooks.start_computer_use_guard_watchdog(&settings).await?;
        }

        let page_readiness_error = hooks
            .wait_for_codex_config_load(debug_port)
            .await
            .err()
            .map(|error| error.to_string());
        let page_ready = page_readiness_error.is_none();
        if page_ready {
            // 页面已确认加载配置，切换锁要守护的「Codex 读到刚写入的配置」目标已达成；
            // 后续注入重试不读写 config/auth 且可能耗时较久，提前放锁避免阻塞 Manager。
            drop(relay_switch_lock.take());
        }
        let mut injection_ready = false;
        let mut startup_degraded = false;
        if settings.enhancements_enabled {
            injection_ready = hooks
                .ensure_injection(debug_port, helper_port, &app_dir)
                .await;
            if injection_ready {
                // 注入成功证明 Codex 页面已可被桥接；无论前面的 readiness 探测是否
                // 超时，供应商切换锁都不再需要保护后续非配置操作。立即释放，避免
                // Local Storage 清理或 watchdog 启动期间阻塞 Manager 的供应商切换。
                drop(relay_switch_lock.take());
                // 注入成功后页面已加载，此时可以通过 CDP 清理 Electron Local Storage
                // 中残留的带后缀模型名，避免模型选择器继续显示废弃项。
                hooks
                    .sanitize_local_storage_model_suffixes(debug_port)
                    .await;
                hooks.start_bridge_watchdog(debug_port, helper_port).await?;
            } else {
                startup_degraded = true;
            }
        }
        if !page_ready && !injection_ready {
            // Codex 已成功启动。CDP 就绪超时仅表示可选页面增强未能确认；
            // 若将其视为致命错误，会终止用户正在使用的 Codex，表现为无提示闪退。
            // 保持进程运行，并显式记录降级状态。
            startup_degraded = true;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.startup_degraded",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port,
                    "page_readiness_error": page_readiness_error.as_deref(),
                    "injection_ready": injection_ready
                }),
            );
        }
        // A readiness timeout leaves the process state uncertain.  Check liveness before
        // publishing an active status in that case, including when a later injection succeeds;
        // a page that was already confirmed ready is itself sufficient evidence of liveness.
        if (!page_ready || startup_degraded) && !hooks.codex_process_is_alive(&launch).await {
            anyhow::bail!("Codex exited before startup completed");
        }
        if startup_degraded {
            let message = page_readiness_error
                .as_deref()
                .map(|error| format!("Codex launched; page readiness is degraded: {error}"))
                .unwrap_or_else(|| {
                    "Codex launched; Codex++ enhancements are still waiting for the page bridge."
                        .to_string()
                });
            let degraded = launch_status(
                "running_degraded",
                &message,
                debug_port,
                helper_port,
                &app_dir,
            );
            options.status_store.save_latest(&degraded)?;
            launch_started_at_ms = Some(degraded.started_at_ms);
            hooks.write_status("running_degraded").await;
        } else {
            let status = launch_status(
                "running",
                "Codex++ launcher ready",
                debug_port,
                helper_port,
                &app_dir,
            );
            options.status_store.save_latest(&status)?;
            launch_started_at_ms = Some(status.started_at_ms);
            hooks.write_status("running").await;
        }
        drop(relay_switch_lock.take());

        Ok(LaunchHandle {
            debug_port,
            helper_port,
            app_dir: app_dir.clone(),
            launch,
            status_store: status_store.clone(),
            started_at_ms: launch_started_at_ms.context("启动状态未记录时间戳")?,
            helper_started,
            hooks: Arc::clone(&hooks),
        })
    }
    .await;

    match result {
        Ok(handle) => Ok(handle),
        Err(mut error) => {
            if helper_started {
                hooks.shutdown_helper(helper_port).await;
            }
            if let Some(launch) = &launched {
                if let Err(termination_error) = hooks.terminate_codex(launch).await {
                    error = anyhow::anyhow!(
                        "{error:#}; additionally failed to terminate Codex: {termination_error:#}"
                    );
                }
            }
            drop(relay_switch_lock.take());
            let message = error.to_string();
            let failure = launch_status("failed", &message, debug_port, helper_port, &app_dir);
            let _ = status_store.save_latest(&failure);
            hooks.write_status("failed").await;
            Err(error)
        }
    }
}

fn relay_protocol_proxy_enabled(settings: &BackendSettings) -> bool {
    settings.active_relay_uses_protocol_proxy()
}

pub fn should_apply_active_relay_profile_at_launch(settings: &BackendSettings) -> bool {
    if !settings.relay_profiles_enabled {
        return false;
    }
    let profile = settings.active_relay_profile();
    profile.relay_mode != crate::settings::RelayMode::Official || profile.official_mix_api_key
}

fn select_native_menu_inspector_port(debug_port: u16) -> u16 {
    let requested = debug_port.saturating_add(100);
    crate::ports::select_platform_loopback_port(requested)
}

fn start_native_menu_localizer(inspector_port: u16) {
    if inspector_port == 0 {
        return;
    }
    tokio::spawn(async move {
        if let Err(error) = crate::native_menu::install_native_menu_localizer(inspector_port).await
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "native_menu.localization_failed",
                serde_json::json!({
                    "inspector_port": inspector_port,
                    "message": error.to_string()
                }),
            );
        }
    });
}

#[cfg(windows)]
fn apply_codexplusplus_window_icon_after_launch(process_id: u32) {
    let icon_resource_path =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("codex-plus-plus.exe"));
    tokio::spawn(async move {
        for attempt in 1..=30 {
            if crate::windows_apply_codexplusplus_icon_to_process_window(
                process_id,
                icon_resource_path.clone(),
            ) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if attempt == 30 {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.window_icon.apply_failed",
                    serde_json::json!({
                        "process_id": process_id,
                        "icon_resource_path": icon_resource_path.to_string_lossy()
                    }),
                );
            }
        }
    });
}

#[cfg(not(windows))]
fn apply_codexplusplus_window_icon_after_launch(_process_id: u32) {}

pub trait IntoLaunchHooks {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks>;
}

impl<T> IntoLaunchHooks for &T
where
    T: LaunchHooks + Clone + 'static,
{
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        Arc::new(self.clone())
    }
}

impl IntoLaunchHooks for Arc<dyn LaunchHooks> {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        self
    }
}

impl IntoLaunchHooks for DefaultLaunchHooks {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        Arc::new(self)
    }
}

impl DefaultLaunchHooks {
    pub fn shared() -> Arc<dyn LaunchHooks> {
        Arc::new(Self::default())
    }
}

fn helper_bind_host() -> String {
    std::env::var("CODEX_PLUS_HELPER_BIND")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

#[async_trait(?Send)]
impl LaunchHooks for DefaultLaunchHooks {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf> {
        crate::app_paths::resolve_codex_app_dir_with_saved(
            app_dir,
            Some(settings.codex_app_path.as_str()),
        )
        .ok_or_else(|| anyhow::anyhow!("Codex App directory not found"))
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        crate::ports::select_packaged_codex_debug_port(requested)
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        crate::ports::select_platform_loopback_port(requested)
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        SettingsStore::default().load()
    }

    async fn run_provider_sync(&self, _codex_home: &Path) -> anyhow::Result<()> {
        anyhow::bail!("provider sync requires launcher hooks with codex-plus-data integration")
    }

    async fn apply_active_relay_profile(
        &self,
        settings: &BackendSettings,
        codex_home: &Path,
    ) -> anyhow::Result<()> {
        if !settings.relay_profiles_enabled {
            return Ok(());
        }
        let profile = settings.active_relay_profile();
        let common_config = crate::relay_config::normalize_config_text(
            &[
                settings.relay_common_config_contents.as_str(),
                settings.relay_context_config_contents.as_str(),
            ]
            .into_iter()
            .map(str::trim)
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        );
        if profile.relay_mode == crate::settings::RelayMode::Official
            && !profile.official_mix_api_key
        {
            let auth_contents = (!profile.auth_contents.trim().is_empty())
                .then_some(profile.auth_contents.as_str());
            crate::relay_config::clear_relay_config_to_home_with_auth_and_computer_use_guard(
                codex_home,
                auth_contents,
                settings.computer_use_guard_enabled,
            )?;
            return Ok(());
        }
        crate::relay_config::apply_relay_profile_to_home_with_switch_rules_and_computer_use_guard(
            codex_home,
            &profile,
            &common_config,
            settings.computer_use_guard_enabled,
        )?;
        Ok(())
    }

    async fn ensure_computer_use_config(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        if !settings.computer_use_guard_enabled {
            return Ok(());
        }
        let home = crate::relay_config::default_codex_home_dir();
        let artifacts = crate::computer_use_guard::resolve_computer_use_guard_artifacts(&home)?;
        crate::computer_use_guard::ensure_computer_use_config_with_artifacts(&home, &artifacts)?;
        *self.computer_use_guard_artifacts.lock().await = Some(artifacts);
        Ok(())
    }

    async fn ensure_plugin_marketplace_config(
        &self,
        settings: &BackendSettings,
        relay_switch_lock: &crate::relay_switch::RelaySwitchLockGuard,
    ) -> anyhow::Result<()> {
        if !settings.codex_app_plugin_marketplace_unlock {
            return Ok(());
        }
        let home = crate::relay_config::default_codex_home_dir();
        match crate::plugin_marketplace::ensure_openai_curated_marketplace_config_with_lock(
            &home,
            relay_switch_lock,
        ) {
            Ok(configured) => {
                if configured {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.openai_curated_marketplace_configured",
                        serde_json::json!({
                            "home": home,
                        }),
                    );
                }
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.openai_curated_marketplace_config_failed",
                    serde_json::json!({
                        "home": home,
                        "message": error.to_string(),
                    }),
                );
            }
        }
        match crate::plugin_marketplace::ensure_role_specific_plugins_marketplace_config_with_lock(
            &home,
            relay_switch_lock,
        ) {
            Ok(configured) => {
                if configured {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.role_specific_plugins_marketplace_configured",
                        serde_json::json!({
                            "home": home,
                        }),
                    );
                }
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.role_specific_plugins_marketplace_config_failed",
                    serde_json::json!({
                        "home": home,
                        "message": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        let bind_host = helper_bind_host();
        let listener = tokio::net::TcpListener::bind((bind_host.as_str(), helper_port))
            .await
            .with_context(|| {
                format!("failed to bind helper runtime on {bind_host}:{helper_port}")
            })?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.listening",
            serde_json::json!({
                "helper_port": helper_port,
                "bind_host": bind_host,
                "address": format!("http://{bind_host}:{helper_port}")
            }),
        );
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let active_connections = Arc::new(AtomicUsize::new(0));
        let task_active_connections = Arc::clone(&active_connections);
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    _ = connections.join_next(), if !connections.is_empty() => {}
                    accepted = listener.accept() => {
                        if let Ok((stream, addr)) = accepted {
                            let active_connection = ActiveHelperConnection::new(Arc::clone(
                                &task_active_connections,
                            ));
                            connections.spawn(async move {
                                let _active_connection = active_connection;
                                let _ = handle_helper_connection(stream, Some(addr)).await;
                            });
                        }
                    }
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        *self.helper.lock().await = Some(HelperRuntime {
            shutdown: shutdown_tx,
            task,
            active_connections,
        });
        Ok(())
    }

    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        settings: &BackendSettings,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch> {
        if settings.enhancements_enabled {
            let home = crate::relay_config::default_codex_home_dir();
            crate::codex_app_state::prepare_projectless_main_window_nonfatal(
                &home,
                "launcher.prelaunch",
            );
        }
        let native_menu_localization_enabled = settings.codex_app_native_menu_localization;
        let native_menu_inspector_port =
            native_menu_localization_enabled.then(|| select_native_menu_inspector_port(debug_port));
        let launch_extra_args = codex_extra_args_for_launch(settings, extra_args);
        #[cfg(windows)]
        {
            let activation = if let Some(inspector_port) = native_menu_inspector_port {
                build_packaged_activation_with_native_menu_inspector(
                    app_dir,
                    debug_port,
                    inspector_port,
                    &launch_extra_args,
                )
            } else {
                build_packaged_activation(app_dir, debug_port, &launch_extra_args)
            };
            if let Some(activation) = activation {
                let CodexLaunch::PackagedActivation {
                    app_user_model_id,
                    arguments,
                    ..
                } = &activation
                else {
                    unreachable!();
                };
                let baseline = match windows_packaged_activation_baseline() {
                    Ok(identities) => Some(identities),
                    Err(error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.packaged_activation_baseline_unavailable",
                            serde_json::json!({
                                "message": error.to_string()
                            }),
                        );
                        None
                    }
                };
                let (process_id, process_handle) =
                    activate_packaged_app_with_process_handle(app_user_model_id, arguments).await?;
                let packaged_process = match process_handle {
                    Ok(Some(handle)) => match packaged_process_cleanup_action(
                        baseline.as_deref(),
                        Some(handle.identity()),
                    ) {
                        PackagedProcessCleanupAction::SkipExisting => {
                            WindowsPackagedProcess::Existing(handle)
                        }
                        PackagedProcessCleanupAction::WaitForExitWithoutTermination => {
                            WindowsPackagedProcess::Unconfirmed {
                                process_id,
                                handle: Some(handle),
                            }
                        }
                    },
                    Ok(None) => WindowsPackagedProcess::Unconfirmed {
                        process_id,
                        handle: None,
                    },
                    Err(error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.packaged_activation_identity_unavailable",
                            serde_json::json!({
                                "process_id": process_id,
                                "message": error.to_string()
                            }),
                        );
                        WindowsPackagedProcess::Unconfirmed {
                            process_id,
                            handle: None,
                        }
                    }
                };
                *self.packaged_process.lock().await = Some(packaged_process);
                apply_codexplusplus_window_icon_after_launch(process_id);
                if let Some(inspector_port) = native_menu_inspector_port {
                    start_native_menu_localizer(inspector_port);
                }
                return Ok(match activation {
                    CodexLaunch::PackagedActivation {
                        app_user_model_id,
                        arguments,
                        ..
                    } => CodexLaunch::PackagedActivation {
                        app_user_model_id,
                        arguments,
                        process_id: Some(process_id),
                    },
                    CodexLaunch::Process { .. } => unreachable!(),
                });
            }
        }

        if app_dir.extension().and_then(|value| value.to_str()) == Some("app") {
            let cleanup_policy = if is_macos_app_running(app_dir).await? {
                MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning
            } else {
                MacosCleanupPolicy::QuitIfNotPreviouslyRunning
            };
            let command = if let Some(inspector_port) = native_menu_inspector_port {
                build_macos_open_command_with_native_menu_inspector(
                    app_dir,
                    debug_port,
                    inspector_port,
                    &launch_extra_args,
                )
            } else {
                build_macos_open_command(app_dir, debug_port, &launch_extra_args)
            };
            let executable = command
                .first()
                .ok_or_else(|| anyhow::anyhow!("macOS open command is empty"))?;
            let child = Command::new(executable)
                .args(&command[1..])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("failed to launch macOS Codex app")?;
            *self.child.lock().await = Some(child);
            if let Some(inspector_port) = native_menu_inspector_port {
                start_native_menu_localizer(inspector_port);
            }
            return Ok(CodexLaunch::Process {
                command,
                wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                macos_cleanup_policy: Some(cleanup_policy),
            });
        }

        let command = if let Some(inspector_port) = native_menu_inspector_port {
            build_codex_command_with_native_menu_inspector(
                app_dir,
                debug_port,
                inspector_port,
                &launch_extra_args,
            )
        } else {
            build_codex_command(app_dir, debug_port, &launch_extra_args)
        };
        let executable = command
            .first()
            .ok_or_else(|| anyhow::anyhow!("Codex command is empty"))?;
        let mut child_command = Command::new(executable);
        child_command
            .args(&command[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "linux")]
        child_command.process_group(0);
        #[cfg(windows)]
        child_command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let child = child_command
            .spawn()
            .with_context(|| format!("failed to launch Codex executable {executable}"))?;
        #[cfg(windows)]
        let mut child = child;
        #[cfg(windows)]
        {
            let process_id = child.id();
            let process_handle = match (process_id, child.try_wait()) {
                (Some(process_id), Ok(None)) => match open_windows_process_handle(process_id, true)
                {
                    Ok(Some(handle)) if matches!(child.try_wait(), Ok(None)) => Some(handle),
                    Ok(Some(_)) | Ok(None) => None,
                    Err(error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.direct_process_identity_unavailable",
                            serde_json::json!({
                                "process_id": process_id,
                                "message": error.to_string()
                            }),
                        );
                        None
                    }
                },
                (_, Ok(Some(_))) | (None, Ok(None)) => None,
                (_, Err(error)) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.direct_process_state_unavailable",
                        serde_json::json!({
                            "process_id": process_id,
                            "message": error.to_string()
                        }),
                    );
                    None
                }
            };
            *self.launched_process_handle.lock().await = process_handle;
        }
        #[cfg(target_os = "linux")]
        {
            *self.launched_process_group.lock().await = child.id();
        }
        *self.child.lock().await = Some(child);
        if let Some(inspector_port) = native_menu_inspector_port {
            start_native_menu_localizer(inspector_port);
        }
        Ok(CodexLaunch::Process {
            command,
            wait_strategy: ProcessWaitStrategy::TrackedChild,
            macos_cleanup_policy: None,
        })
    }

    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        retry_injection(debug_port, helper_port).await
    }
    async fn start_bridge_watchdog(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            #[cfg(windows)]
            let pet_cursor_task = tokio::spawn(run_pet_real_mouse_cursor_driver(debug_port));
            let mut observed_browser_id: Option<String> = None;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    _ = interval.tick() => {
                        let current_browser_id = match crate::cdp::browser_identity(debug_port).await {
                            Ok(identity) => identity.browser_id().ok(),
                            Err(_) => None,
                        };
                        let identity_changed = current_browser_id
                            .as_deref()
                            .is_some_and(|current| {
                                browser_identity_changed(observed_browser_id.as_deref(), current)
                            });
                        if let Some(current) = current_browser_id {
                            observed_browser_id = Some(current);
                        }
                        let (pet_result, _) = tokio::join!(
                            sync_pet_real_mouse_overlay(debug_port, helper_port),
                            check_and_reinject_bridge_inner(
                                debug_port,
                                helper_port,
                                identity_changed,
                            ),
                        );
                        record_pet_overlay_sync_result(debug_port, helper_port, pet_result);
                    }
                }
            }
            #[cfg(windows)]
            {
                pet_cursor_task.abort();
                let _ = pet_cursor_task.await;
            }
        });
        if let Some(runtime) = self
            .bridge_watchdog
            .lock()
            .await
            .replace(BridgeWatchdogRuntime { shutdown, task })
        {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        Ok(())
    }

    async fn start_computer_use_guard_watchdog(
        &self,
        settings: &BackendSettings,
    ) -> anyhow::Result<()> {
        #[cfg(windows)]
        {
            if !settings.computer_use_guard_enabled {
                return Ok(());
            }
            let home = crate::relay_config::default_codex_home_dir();
            let artifacts = self.computer_use_guard_artifacts.lock().await.clone();
            let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                run_post_launch_computer_use_guard(home, artifacts, &mut shutdown_rx).await;
            });
            if let Some(runtime) = self
                .computer_use_guard_watchdog
                .lock()
                .await
                .replace(ComputerUseGuardWatchdogRuntime { shutdown, task })
            {
                let _ = runtime.shutdown.send(());
                let _ = runtime.task.await;
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = &settings;
            let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                            crate::computer_use_guard::kill_orphaned_computer_use_processes();
                        }
                    }
                }
            });
            if let Some(runtime) = self
                .computer_use_guard_watchdog
                .lock()
                .await
                .replace(ComputerUseGuardWatchdogRuntime { shutdown, task })
            {
                let _ = runtime.shutdown.send(());
                let _ = runtime.task.await;
            }
        }
        Ok(())
    }

    async fn write_status(&self, _status: &str) {}

    async fn codex_process_is_alive(&self, launch: &CodexLaunch) -> bool {
        match launch {
            CodexLaunch::Process {
                wait_strategy,
                command,
                ..
            } => {
                // `open -W` on macOS is an external waiter and may hand off to an already
                // running app; query the app itself instead of trusting the waiter child.
                if *wait_strategy == ProcessWaitStrategy::ExternalWaitCommand {
                    #[cfg(target_os = "macos")]
                    if let Some(app_dir) = macos_app_dir_from_open_command(command) {
                        return is_macos_app_running(&app_dir).await.unwrap_or(true);
                    }
                    // On platforms without an app-state query, stay conservative.
                    return true;
                }
                #[cfg(not(target_os = "macos"))]
                let _ = command;
                let mut child = self.child.lock().await;
                match child.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(None) => true,
                        Ok(Some(_)) => false,
                        // A transient OS query failure must not make us terminate a healthy app.
                        Err(_) => true,
                    },
                    // The child may already have been reaped by another lifecycle path.  We
                    // cannot attribute a process-list match to this launch reliably (there may
                    // be other Codex instances), so stay conservative and let the normal exit
                    // waiter publish the terminal status instead of rejecting this launch.
                    None => true,
                }
            }
            CodexLaunch::PackagedActivation { process_id, .. } => {
                #[cfg(windows)]
                {
                    let process = self.packaged_process.lock().await.clone();
                    let known_process = match process {
                        Some(WindowsPackagedProcess::Existing(handle))
                        | Some(WindowsPackagedProcess::Unconfirmed {
                            handle: Some(handle),
                            ..
                        }) => Some(handle),
                        Some(WindowsPackagedProcess::Unconfirmed {
                            process_id,
                            handle: None,
                        }) => open_windows_process_handle(process_id, false)
                            .ok()
                            .flatten(),
                        None => process_id.as_ref().and_then(|process_id| {
                            open_windows_process_handle(*process_id, false)
                                .ok()
                                .flatten()
                        }),
                    };
                    match known_process {
                        Some(handle) => windows_process_handle_has_exited(&handle)
                            .map(|exited| !exited)
                            .unwrap_or(true),
                        // We cannot prove that an unconfirmed packaged process is gone when the
                        // OS query itself is unavailable.  Stay conservative and let the normal
                        // exit waiter publish the terminal status later.
                        None => true,
                    }
                }
                #[cfg(not(windows))]
                {
                    // Packaged activation is Windows-only in production.  Keep test/custom
                    // implementations conservative on other platforms.
                    let _ = process_id;
                    true
                }
            }
        }
    }

    async fn wait_for_codex_exit(&self, launch: &CodexLaunch) -> anyhow::Result<()> {
        match launch {
            CodexLaunch::Process { .. } => {
                if let Some(mut child) = self.child.lock().await.take() {
                    let _ = child.wait().await;
                }
            }
            CodexLaunch::PackagedActivation { process_id, .. } => {
                #[cfg(windows)]
                {
                    let process = self.packaged_process.lock().await.clone();
                    match process {
                        Some(WindowsPackagedProcess::Existing(handle)) => {
                            wait_for_windows_process_handle(handle).await?;
                        }
                        Some(WindowsPackagedProcess::Unconfirmed { process_id, handle }) => {
                            if let Some(handle) = handle {
                                wait_for_windows_process_handle(handle).await?;
                            } else {
                                wait_for_windows_process_id(process_id).await?;
                            }
                        }
                        None => {
                            if let Some(process_id) = process_id {
                                wait_for_windows_process_id(*process_id).await?;
                            }
                        }
                    }
                }
                #[cfg(not(windows))]
                if let Some(process_id) = process_id {
                    wait_for_windows_process_id(*process_id).await?;
                }
            }
        }
        let mut exit_observation = ProcessExitObservation::new();
        loop {
            let has_codex_process = !crate::watcher::find_codex_processes().is_empty();
            if exit_observation.observe(has_codex_process) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Ok(())
    }

    async fn shutdown_helper(&self, _helper_port: u16) {
        if let Some(runtime) = self.computer_use_guard_watchdog.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        if let Some(runtime) = self.bridge_watchdog.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        if let Some(runtime) = self.helper.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
    }

    async fn terminate_codex(&self, launch: &CodexLaunch) -> anyhow::Result<()> {
        let mut cleanup_errors = Vec::new();
        match launch {
            CodexLaunch::Process {
                wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                command,
                macos_cleanup_policy,
            } => {
                if let (Some(app_dir), Some(cleanup_policy)) = (
                    macos_app_dir_from_open_command(command),
                    *macos_cleanup_policy,
                ) {
                    if cleanup_policy == MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning {
                        if let Some(mut child) = self.child.lock().await.take() {
                            record_cleanup_result(
                                &mut cleanup_errors,
                                "terminate macOS open waiter",
                                terminate_tracked_child(&mut child).await,
                            );
                        }
                    } else {
                        record_cleanup_result(
                            &mut cleanup_errors,
                            "request macOS app quit",
                            run_macos_cleanup_command(&app_dir, cleanup_policy).await,
                        );
                        let mut app_exit_confirmed = false;
                        let mut child = self.child.lock().await.take();
                        if let Some(child) = child.as_mut() {
                            let wait_result = wait_for_tracked_child_exit(child).await;
                            app_exit_confirmed = wait_result.is_ok();
                            record_cleanup_result(
                                &mut cleanup_errors,
                                "wait for macOS open waiter",
                                wait_result,
                            );
                        }
                        let app_exit_result = wait_for_macos_app_exit(&app_dir).await;
                        app_exit_confirmed |= app_exit_result.is_ok();
                        record_cleanup_result(
                            &mut cleanup_errors,
                            "confirm macOS app exit",
                            app_exit_result,
                        );
                        if !app_exit_confirmed {
                            let waiter = child
                                .as_mut()
                                .map(wait_for_tracked_child_exit_without_timeout);
                            if let Some(error) = wait_for_macos_exit_after_bounded_failures_with(
                                waiter,
                                || is_macos_app_running(&app_dir),
                                std::time::Duration::from_millis(100),
                            )
                            .await
                            {
                                record_cleanup_result(
                                    &mut cleanup_errors,
                                    "wait for macOS open waiter until app exit",
                                    Err(error),
                                );
                            }
                        }
                    }
                } else if let Some(mut child) = self.child.lock().await.take() {
                    record_cleanup_result(
                        &mut cleanup_errors,
                        "terminate external Codex process",
                        terminate_tracked_child(&mut child).await,
                    );
                }
            }
            CodexLaunch::Process { .. } => {
                #[cfg(windows)]
                {
                    let process_handle = self.launched_process_handle.lock().await.take();
                    if let Some(handle) = process_handle {
                        record_cleanup_result(
                            &mut cleanup_errors,
                            "clean up confirmed Windows Codex process tree",
                            cleanup_owned_windows_process(handle).await,
                        );
                    } else {
                        if let Some(mut child) = self.child.lock().await.take() {
                            record_cleanup_result(
                                &mut cleanup_errors,
                                "wait for unconfirmed Windows Codex process",
                                wait_for_tracked_child_exit_confirmed(&mut child).await,
                            );
                        }
                        cleanup_errors.push(
                            "Windows Codex process identity was not confirmed; no process was terminated"
                                .to_string(),
                        );
                    }
                    if let Some(mut child) = self.child.lock().await.take() {
                        record_cleanup_result(
                            &mut cleanup_errors,
                            "reap terminated Windows Codex process",
                            wait_for_tracked_child_exit_confirmed(&mut child).await,
                        );
                    }
                }
                #[cfg(not(windows))]
                if let Some(mut child) = self.child.lock().await.take() {
                    record_cleanup_result(
                        &mut cleanup_errors,
                        "terminate tracked Codex process",
                        terminate_tracked_child(&mut child).await,
                    );
                }
                #[cfg(target_os = "linux")]
                if let Some(process_group) = self.launched_process_group.lock().await.take() {
                    record_cleanup_result(
                        &mut cleanup_errors,
                        "terminate launched Linux Codex process group",
                        terminate_linux_process_group_and_wait(process_group).await,
                    );
                }
            }
            CodexLaunch::PackagedActivation {
                process_id: Some(process_id),
                ..
            } => {
                #[cfg(windows)]
                {
                    let process = self.packaged_process.lock().await.take();
                    let (action, unconfirmed_handle, unconfirmed_process_id) = match process {
                        Some(WindowsPackagedProcess::Existing(_)) => {
                            (PackagedProcessCleanupAction::SkipExisting, None, None)
                        }
                        Some(WindowsPackagedProcess::Unconfirmed { process_id, handle }) => (
                            PackagedProcessCleanupAction::WaitForExitWithoutTermination,
                            handle,
                            Some(process_id),
                        ),
                        None => (
                            PackagedProcessCleanupAction::WaitForExitWithoutTermination,
                            None,
                            Some(*process_id),
                        ),
                    };
                    record_cleanup_result(
                        &mut cleanup_errors,
                        "clean up Windows packaged process",
                        cleanup_packaged_process_with(action, || async move {
                            if let Some(handle) = unconfirmed_handle {
                                wait_for_windows_process_handle_confirmed(handle).await;
                            } else {
                                wait_for_windows_process_exit_confirmed(
                                    unconfirmed_process_id.unwrap_or(*process_id),
                                )
                                .await;
                            }
                            Ok(())
                        })
                        .await,
                    );
                }
                #[cfg(not(windows))]
                cleanup_errors.push(format!(
                    "Windows packaged process {process_id} cannot be cleaned up on this platform"
                ));
            }
            CodexLaunch::PackagedActivation {
                process_id: None, ..
            } => {}
        }
        finish_cleanup(cleanup_errors)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackagedProcessCleanupAction {
    SkipExisting,
    WaitForExitWithoutTermination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsProcessIdentity {
    process_id: u32,
    creation_time: u64,
}

#[cfg(windows)]
#[derive(Clone)]
struct WindowsProcessHandle(Arc<WindowsProcessHandleInner>);

#[cfg(windows)]
struct WindowsProcessHandleInner {
    raw_handle: usize,
    identity: WindowsProcessIdentity,
}

#[cfg(windows)]
impl Drop for WindowsProcessHandleInner {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};

        let handle = HANDLE(self.raw_handle as *mut core::ffi::c_void);
        let _ = unsafe { CloseHandle(handle) };
    }
}

#[cfg(windows)]
impl WindowsProcessHandle {
    fn identity(&self) -> WindowsProcessIdentity {
        self.0.identity
    }

    fn raw_handle(&self) -> windows::Win32::Foundation::HANDLE {
        windows::Win32::Foundation::HANDLE(self.0.raw_handle as *mut core::ffi::c_void)
    }
}

#[cfg(windows)]
#[derive(Clone)]
enum WindowsPackagedProcess {
    Existing(WindowsProcessHandle),
    Unconfirmed {
        process_id: u32,
        handle: Option<WindowsProcessHandle>,
    },
}

impl WindowsProcessIdentity {
    pub const fn new(process_id: u32, creation_time: u64) -> Self {
        Self {
            process_id,
            creation_time,
        }
    }

    pub const fn process_id(self) -> u32 {
        self.process_id
    }

    pub const fn creation_time(self) -> u64 {
        self.creation_time
    }
}

pub fn packaged_process_cleanup_action(
    baseline: Option<&[WindowsProcessIdentity]>,
    launched: Option<WindowsProcessIdentity>,
) -> PackagedProcessCleanupAction {
    match (baseline, launched) {
        (Some(processes), Some(launched)) if processes.contains(&launched) => {
            PackagedProcessCleanupAction::SkipExisting
        }
        (Some(_), Some(_)) => PackagedProcessCleanupAction::WaitForExitWithoutTermination,
        _ => PackagedProcessCleanupAction::WaitForExitWithoutTermination,
    }
}

fn packaged_activation_baseline_identities_with<F>(
    processes: &[(u32, &str)],
    mut identify: F,
) -> anyhow::Result<Vec<WindowsProcessIdentity>>
where
    F: FnMut(u32) -> anyhow::Result<Option<WindowsProcessIdentity>>,
{
    let mut identities = Vec::new();
    for process_id in crate::watcher::packaged_activation_baseline_process_ids(processes) {
        let Some(identity) = identify(process_id)? else {
            continue;
        };
        if identity.process_id() != process_id {
            anyhow::bail!(
                "Windows process identity mismatch: expected {process_id}, observed {}",
                identity.process_id()
            );
        }
        identities.push(identity);
    }
    Ok(identities)
}

#[cfg(windows)]
fn windows_packaged_activation_baseline() -> anyhow::Result<Vec<WindowsProcessIdentity>> {
    let processes = crate::windows_integration::try_enumerate_processes()?;
    let process_names = processes
        .iter()
        .map(|process| (process.process_id, process.exe_file.as_str()))
        .collect::<Vec<_>>();
    packaged_activation_baseline_identities_with(&process_names, |process_id| {
        open_windows_process_handle(process_id, false)
            .map(|handle| handle.map(|handle| handle.identity()))
    })
}

async fn cleanup_packaged_process_with<Wait, WaitFuture>(
    action: PackagedProcessCleanupAction,
    wait_unconfirmed: Wait,
) -> anyhow::Result<()>
where
    Wait: FnOnce() -> WaitFuture,
    WaitFuture: std::future::Future<Output = anyhow::Result<()>>,
{
    match action {
        PackagedProcessCleanupAction::SkipExisting => Ok(()),
        PackagedProcessCleanupAction::WaitForExitWithoutTermination => {
            wait_unconfirmed().await?;
            anyhow::bail!(
                "Windows packaged process ownership was not confirmed; no process was terminated"
            )
        }
    }
}

async fn cleanup_confirmed_process_tree_with<
    T,
    Terminate,
    TerminateFuture,
    WaitRoot,
    WaitRootFuture,
    WaitDescendant,
    WaitDescendantFuture,
>(
    root: T,
    descendants: anyhow::Result<Vec<T>>,
    terminate_root: Terminate,
    wait_root: WaitRoot,
    mut wait_descendant: WaitDescendant,
) -> anyhow::Result<()>
where
    T: Clone,
    Terminate: FnOnce(T) -> TerminateFuture,
    TerminateFuture: std::future::Future<Output = anyhow::Result<()>>,
    WaitRoot: FnOnce(T) -> WaitRootFuture,
    WaitRootFuture: std::future::Future<Output = ()>,
    WaitDescendant: FnMut(T) -> WaitDescendantFuture,
    WaitDescendantFuture: std::future::Future<Output = ()>,
{
    let descendants = match descendants {
        Ok(descendants) => descendants,
        Err(error) => {
            wait_root(root).await;
            return Err(error).context(
                "Windows process tree ownership could not be confirmed; no process was terminated",
            );
        }
    };

    let root_confirmation = root.clone();
    let termination_result = terminate_root(root).await;
    if termination_result.is_err() {
        wait_root(root_confirmation).await;
    }
    for descendant in descendants {
        wait_descendant(descendant).await;
    }
    termination_result
}

fn record_cleanup_result(errors: &mut Vec<String>, operation: &str, result: anyhow::Result<()>) {
    if let Err(error) = result {
        errors.push(format!("{operation}: {error:#}"));
    }
}

fn finish_cleanup(errors: Vec<String>) -> anyhow::Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

async fn handle_helper_connection(
    mut stream: tokio::net::TcpStream,
    remote_addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let request = match read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }))?;
            write_http_response(
                &mut stream,
                error.status(),
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let request_headers = String::from_utf8_lossy(&request.headers);
    let request_line = request_headers.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or_default();
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let request_user_agent = header_value_from_headers(&request_headers, "user-agent");
    let request_content_type = header_value_from_headers(&request_headers, "content-type");
    let remote_addr_text = remote_addr.map(|addr| addr.to_string());

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "helper.request",
        serde_json::json!({
            "method": method,
            "path": path,
            "request_line": request_line,
            "remote_addr": remote_addr_text,
            "body_bytes": request.body.len()
        }),
    );

    if crate::protocol_proxy::is_audio_transcriptions_proxy_path(path) && method == "POST" {
        return handle_audio_transcriptions_proxy_connection(
            &mut stream,
            &request.body,
            request_content_type.as_deref(),
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    let request_body = String::from_utf8_lossy(&request.body);
    if crate::protocol_proxy::is_responses_proxy_path(path) && method == "POST" {
        return handle_protocol_proxy_connection(
            &mut stream,
            &request_body,
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_chat_completions_proxy_path(path) && method == "POST" {
        return handle_chat_completions_proxy_connection(
            &mut stream,
            &request_body,
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_models_proxy_path(path) && matches!(method, "GET" | "OPTIONS") {
        return handle_models_proxy_connection(
            &mut stream,
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }

    let (status, body, content_type, log_event) = if path == "/backend/status"
        && matches!(method, "GET" | "POST" | "OPTIONS")
    {
        (
            "200 OK".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "ok",
                "message": "后端已连接",
                "version": crate::version::VERSION,
                "transport": "http-helper"
            }))?,
            "application/json; charset=utf-8".to_string(),
            "helper.backend_status_ok",
        )
    } else if path == "/diagnostics/log" && matches!(method, "POST" | "OPTIONS") {
        if method == "POST" {
            let detail =
                serde_json::from_str::<serde_json::Value>(&request_body).unwrap_or_else(|error| {
                    serde_json::json!({
                        "parse_error": error.to_string(),
                        "raw": request_body
                    })
                });
            let event = detail
                .get("event")
                .and_then(serde_json::Value::as_str)
                .map(sanitize_diagnostic_event)
                .unwrap_or_else(|| "event".to_string());
            let _ =
                crate::diagnostic_log::append_diagnostic_log(&format!("renderer.{event}"), detail);
        }
        (
            "200 OK".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "ok",
                "message": "日志已记录"
            }))?,
            "application/json; charset=utf-8".to_string(),
            "helper.diagnostics_log_ok",
        )
    } else if path == "/overlay/image" && matches!(method, "GET" | "OPTIONS") {
        if method == "OPTIONS" {
            (
                "200 OK".to_string(),
                Vec::new(),
                "application/octet-stream".to_string(),
                "helper.overlay_image_options",
            )
        } else {
            overlay_image_response()
        }
    } else if path == "/dream-skin/image" && matches!(method, "GET" | "OPTIONS") {
        if method == "OPTIONS" {
            (
                "200 OK".to_string(),
                Vec::new(),
                "application/octet-stream".to_string(),
                "helper.dream_skin_image_options",
            )
        } else {
            dream_skin_image_response()
        }
    } else {
        (
            "404 Not Found".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": "未知后端路径"
            }))?,
            "application/json; charset=utf-8".to_string(),
            "helper.unknown_path",
        )
    };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        log_event,
        serde_json::json!({
            "method": method,
            "path": path,
            "status": status,
            "remote_addr": remote_addr_text
        }),
    );
    let response = if method == "OPTIONS" {
        format!(
            "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
    };
    stream.write_all(response.as_bytes()).await?;
    if method != "OPTIONS" {
        stream.write_all(&body).await?;
    }
    stream.shutdown().await?;
    Ok(())
}

fn overlay_image_response() -> (String, Vec<u8>, String, &'static str) {
    let not_found = || {
        (
            "404 Not Found".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": "图片覆盖层未启用或图片不可用"
            }))
            .unwrap_or_default(),
            "application/json; charset=utf-8".to_string(),
            "helper.overlay_image_not_found",
        )
    };
    let settings = SettingsStore::default().load().unwrap_or_default();
    if !settings.codex_app_image_overlay_enabled {
        return not_found();
    }
    let image_path = PathBuf::from(settings.codex_app_image_overlay_path.trim());
    if image_path.as_os_str().is_empty() || !image_path.is_file() {
        return not_found();
    }
    let Some(content_type) = overlay_image_content_type(&image_path) else {
        return not_found();
    };
    match std::fs::read(&image_path) {
        Ok(bytes) => (
            "200 OK".to_string(),
            bytes,
            content_type.to_string(),
            "helper.overlay_image_ok",
        ),
        Err(_) => not_found(),
    }
}

fn dream_skin_image_response() -> (String, Vec<u8>, String, &'static str) {
    let not_found = || {
        (
            "404 Not Found".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": "皮肤图片未启用或图片不可用"
            }))
            .unwrap_or_default(),
            "application/json; charset=utf-8".to_string(),
            "helper.dream_skin_image_not_found",
        )
    };
    let settings = SettingsStore::default().load().unwrap_or_default();
    if !settings.codex_app_dream_skin_enabled {
        return not_found();
    }
    let image_path = PathBuf::from(settings.codex_app_dream_skin_image_path.trim());
    if image_path.as_os_str().is_empty() || !image_path.is_file() {
        let (content_type, image) = crate::assets::dream_skin_default_image();
        return (
            "200 OK".to_string(),
            image.to_vec(),
            content_type.to_string(),
            "helper.dream_skin_default_image_ok",
        );
    }
    let Some(content_type) = overlay_image_content_type(&image_path) else {
        return not_found();
    };
    match std::fs::read(&image_path) {
        Ok(bytes) => (
            "200 OK".to_string(),
            bytes,
            content_type.to_string(),
            "helper.dream_skin_image_ok",
        ),
        Err(_) => not_found(),
    }
}

#[cfg(windows)]
fn windows_logical_cursor_position() -> anyhow::Result<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_UNAWARE_GDISCALED, SetThreadDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE_GDISCALED) };
    if previous.0.is_null() {
        anyhow::bail!("SetThreadDpiAwarenessContext failed");
    }
    let mut point = POINT::default();
    let result = unsafe { GetCursorPos(&mut point) };
    unsafe {
        SetThreadDpiAwarenessContext(previous);
    }
    result.ok().context("GetCursorPos failed")?;
    Ok((point.x, point.y))
}

fn overlay_image_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

async fn handle_models_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_http_response(
            stream,
            "204 No Content",
            "application/json; charset=utf-8",
            &[],
        )
        .await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let upstream = match crate::protocol_proxy::open_models_proxy_request(request_user_agent).await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(
                &serde_json::json!({                 "status": "failed",                 "message": error.to_string()             }),
            )?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.models_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let status = upstream.status();
    let is_success = upstream.is_success();
    let content_type = if upstream.content_type.is_empty() {
        "application/json; charset=utf-8".to_string()
    } else {
        upstream.content_type.clone()
    };
    let body = upstream.response.bytes().await?.to_vec();
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.models_proxy_ok"
        } else {
            "helper.models_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}
async fn handle_protocol_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &str,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let request_json = serde_json::from_str::<serde_json::Value>(request_body).ok();
    let upstream = match crate::protocol_proxy::open_responses_proxy_request(
        request_body,
        request_user_agent,
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(
                &serde_json::json!({                     "status": "failed",                     "message": error.to_string()                 }),
            )?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.protocol_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };
    if !upstream.is_success() {
        let status = upstream.status();
        let upstream_content_type = upstream.content_type.clone();
        let upstream_body = upstream.response.bytes().await?.to_vec();
        let error = crate::protocol_proxy::responses_error_from_upstream(
            upstream.status_code,
            &upstream_content_type,
            &upstream_body,
        );
        let body = serde_json::to_vec(&error)?;
        write_http_response(stream, &status, "application/json; charset=utf-8", &body).await?;
        log_helper_response(
            "helper.protocol_proxy_upstream_error",
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    if upstream.is_stream {
        write_http_stream_headers(stream, "200 OK", "text/event-stream; charset=utf-8").await?;
        if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
            let mut bytes_stream = upstream.response.bytes_stream();
            while let Some(chunk) = bytes_stream.next().await {
                if let Ok(bytes) = chunk {
                    stream.write_all(&bytes).await?;
                } else {
                    break;
                }
            }
            log_helper_response(
                "helper.protocol_proxy_stream_ok",
                method,
                path,
                "200 OK",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
        let mut converter = request_json
            .as_ref()
            .map(crate::protocol_proxy::ChatSseToResponsesConverter::with_request)
            .unwrap_or_default();
        let mut bytes_stream = upstream.response.bytes_stream();
        let mut stream_failed = false;
        while let Some(chunk) = bytes_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    let converted = converter.push_bytes(&bytes);
                    if !converted.is_empty() {
                        stream.write_all(&converted).await?;
                    }
                }
                Err(error) => {
                    let failed = converter.fail(
                        format!("Stream error: {error}"),
                        Some("stream_error".to_string()),
                    );
                    if !failed.is_empty() {
                        stream.write_all(&failed).await?;
                    }
                    stream_failed = true;
                    break;
                }
            }
        }
        if !stream_failed {
            let tail = converter.finish();
            if !tail.is_empty() {
                stream.write_all(&tail).await?;
            }
        }
        log_helper_response(
            "helper.protocol_proxy_stream_ok",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    let upstream_body = upstream.response.bytes().await?;
    if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
        write_http_response(
            stream,
            "200 OK",
            if upstream.content_type.is_empty() {
                "application/json; charset=utf-8"
            } else {
                &upstream.content_type
            },
            &upstream_body,
        )
        .await?;
        log_helper_response(
            "helper.protocol_proxy_ok",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    let chat_json: serde_json::Value = serde_json::from_slice(&upstream_body)?;
    let response_json = if let Some(request_json) = request_json.as_ref() {
        crate::protocol_proxy::chat_completion_to_response_with_request(chat_json, request_json)?
    } else {
        crate::protocol_proxy::chat_completion_to_response(chat_json)?
    };
    let body = serde_json::to_vec(&response_json)?;
    write_http_response(stream, "200 OK", "application/json; charset=utf-8", &body).await?;
    log_helper_response(
        "helper.protocol_proxy_ok",
        method,
        path,
        "200 OK",
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}
async fn handle_audio_transcriptions_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &[u8],
    request_content_type: Option<&str>,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let upstream = match crate::protocol_proxy::open_audio_transcriptions_proxy_request(
        request_body,
        request_content_type.unwrap_or_default(),
        request_user_agent,
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }))?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.audio_transcriptions_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let status = upstream.status();
    let is_success = upstream.is_success();
    let content_type = if upstream.content_type.is_empty() {
        "application/json; charset=utf-8".to_string()
    } else {
        upstream.content_type.clone()
    };
    let body = upstream.response.bytes().await?.to_vec();
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.audio_transcriptions_proxy_ok"
        } else {
            "helper.audio_transcriptions_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn handle_chat_completions_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &str,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let upstream = match crate::protocol_proxy::open_chat_completions_proxy_request(
        request_body,
        request_user_agent,
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(
                &serde_json::json!({                 "status": "failed",                 "message": error.to_string()             }),
            )?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.chat_completions_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let status = upstream.status();
    let is_success = upstream.is_success();
    let content_type = if upstream.content_type.is_empty() {
        "application/json; charset=utf-8".to_string()
    } else {
        upstream.content_type.clone()
    };
    if upstream.is_stream && is_success {
        write_http_stream_headers(stream, &status, &content_type).await?;
        let mut bytes_stream = upstream.response.bytes_stream();
        while let Some(chunk) = bytes_stream.next().await {
            stream.write_all(&chunk?).await?;
        }
        log_helper_response(
            "helper.chat_completions_proxy_stream_ok",
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    let body = upstream.response.bytes().await?.to_vec();
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.chat_completions_proxy_ok"
        } else {
            "helper.chat_completions_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn write_http_stream_headers(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn log_helper_response(
    event: &str,
    method: &str,
    path: &str,
    status: &str,
    remote_addr_text: Option<String>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        event,
        serde_json::json!({
            "method": method,
            "path": path,
            "status": status,
            "remote_addr": remote_addr_text
        }),
    );
}

#[cfg(test)]
mod computer_use_tests {
    use super::{header_value_from_headers, overlay_image_content_type};
    use std::path::Path;

    #[test]
    fn overlay_image_content_type_accepts_common_images_only() {
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.PNG")),
            Some("image/png")
        );
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.webp")),
            Some("image/webp")
        );
        assert_eq!(overlay_image_content_type(Path::new("overlay.txt")), None);
    }

    #[test]
    fn header_value_from_request_reads_user_agent_case_insensitively() {
        let request = "POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: Codex/26.614\r\nContent-Length: 2\r\n\r\n{}";

        assert_eq!(
            header_value_from_headers(request, "user-agent").as_deref(),
            Some("Codex/26.614")
        );
    }
}

const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_HTTP_ENCODED_BODY_BYTES: usize = 64 * 1024 * 1024;

struct HttpRequest {
    headers: Vec<u8>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpRequestReadError {
    status: &'static str,
    message: String,
}

impl HttpRequestReadError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: "400 Bad Request",
            message: message.into(),
        }
    }

    fn payload_too_large() -> Self {
        Self {
            status: "413 Payload Too Large",
            message: format!("HTTP 请求体超过 {MAX_HTTP_BODY_BYTES} 字节限制"),
        }
    }

    fn status(&self) -> &'static str {
        self.status
    }
}

impl std::fmt::Display for HttpRequestReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpRequestReadError {}

impl From<std::io::Error> for HttpRequestReadError {
    fn from(error: std::io::Error) -> Self {
        Self::bad_request(format!("读取 HTTP 请求失败: {error}"))
    }
}

#[derive(Debug)]
enum HttpBodyFraming {
    Empty,
    ContentLength(usize),
    Chunked,
}

#[derive(Debug)]
enum ChunkedBody {
    Incomplete,
    Complete(Vec<u8>),
}

#[derive(Debug)]
enum ChunkedBodyScan {
    Incomplete,
    Complete,
}

#[derive(Default)]
struct ChunkedScanState {
    position: usize,
    decoded_len: usize,
    complete: bool,
}

impl ChunkedScanState {
    fn advance(&mut self, encoded: &[u8]) -> Result<ChunkedBodyScan, HttpRequestReadError> {
        if self.complete {
            return Ok(ChunkedBodyScan::Complete);
        }
        loop {
            let chunk_start = self.position;
            let Some(line_end_offset) = encoded[chunk_start..]
                .windows(2)
                .position(|window| window == b"\r\n")
            else {
                if encoded.len().saturating_sub(chunk_start) > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
                }
                return Ok(ChunkedBodyScan::Incomplete);
            };
            if line_end_offset > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
            }
            let line_end = chunk_start + line_end_offset;
            let size_text = std::str::from_utf8(&encoded[chunk_start..line_end])
                .map_err(|_| HttpRequestReadError::bad_request("chunk size 不是有效 ASCII"))?;
            let size_token = size_text.split(';').next().unwrap_or_default().trim();
            let chunk_size = usize::from_str_radix(size_token, 16)
                .map_err(|_| HttpRequestReadError::bad_request("chunk size 无效"))?;
            let data_start = line_end + 2;

            if chunk_size == 0 {
                let mut trailer_start = data_start;
                loop {
                    let Some(trailer_end_offset) = encoded[trailer_start..]
                        .windows(2)
                        .position(|window| window == b"\r\n")
                    else {
                        if encoded.len().saturating_sub(data_start) > MAX_HTTP_HEADER_BYTES {
                            return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                        }
                        return Ok(ChunkedBodyScan::Incomplete);
                    };
                    if trailer_start + trailer_end_offset - data_start > MAX_HTTP_HEADER_BYTES {
                        return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                    }
                    if trailer_end_offset == 0 {
                        self.position = trailer_start + 2;
                        self.complete = true;
                        return Ok(ChunkedBodyScan::Complete);
                    }
                    trailer_start += trailer_end_offset + 2;
                }
            }
            let next_decoded_len = self
                .decoded_len
                .checked_add(chunk_size)
                .ok_or_else(HttpRequestReadError::payload_too_large)?;
            if next_decoded_len > MAX_HTTP_BODY_BYTES {
                return Err(HttpRequestReadError::payload_too_large());
            }
            let chunk_end = data_start
                .checked_add(chunk_size)
                .ok_or_else(HttpRequestReadError::payload_too_large)?;
            if encoded.len() < chunk_end + 2 {
                return Ok(ChunkedBodyScan::Incomplete);
            }
            if &encoded[chunk_end..chunk_end + 2] != b"\r\n" {
                return Err(HttpRequestReadError::bad_request("chunk 数据后缺少 CRLF"));
            }
            self.decoded_len = next_decoded_len;
            self.position = chunk_end + 2;
        }
    }
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<HttpRequest, HttpRequestReadError> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0_u8; 4096];
    let mut header_end = None;
    let mut framing = HttpBodyFraming::Empty;
    let mut chunked_scan = ChunkedScanState::default();

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                if end > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("HTTP 请求头过大"));
                }
                framing = http_body_framing(&buffer[..end])?;
            } else if buffer.len() > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("HTTP 请求头过大"));
            }
        }
        if let Some(end) = header_end {
            let body = &buffer[end + 4..];
            if body.len() > MAX_HTTP_ENCODED_BODY_BYTES {
                return Err(HttpRequestReadError::payload_too_large());
            }
            match framing {
                HttpBodyFraming::Empty => break,
                HttpBodyFraming::ContentLength(content_length) => {
                    if content_length > MAX_HTTP_BODY_BYTES {
                        return Err(HttpRequestReadError::payload_too_large());
                    }
                    if body.len() >= content_length {
                        break;
                    }
                }
                HttpBodyFraming::Chunked => match chunked_scan.advance(body)? {
                    ChunkedBodyScan::Incomplete => {}
                    ChunkedBodyScan::Complete => break,
                },
            }
        }
    }

    let header_end =
        header_end.ok_or_else(|| HttpRequestReadError::bad_request("HTTP 请求头不完整"))?;
    let headers = buffer[..header_end].to_vec();
    let encoded_body = &buffer[header_end + 4..];
    let body = match framing {
        HttpBodyFraming::Empty => Vec::new(),
        HttpBodyFraming::ContentLength(content_length) => {
            content_length_body(encoded_body, content_length)?
        }
        HttpBodyFraming::Chunked => match decode_chunked_body(encoded_body)? {
            ChunkedBody::Complete(body) => body,
            ChunkedBody::Incomplete => {
                return Err(HttpRequestReadError::bad_request(
                    "chunked HTTP 请求体不完整",
                ));
            }
        },
    };

    Ok(HttpRequest { headers, body })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn http_body_framing(headers: &[u8]) -> Result<HttpBodyFraming, HttpRequestReadError> {
    let text = String::from_utf8_lossy(headers);
    let mut content_length = None;
    let mut transfer_encoding: Option<String> = None;
    for line in text.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| HttpRequestReadError::bad_request("Content-Length 无效"))?;
            if content_length
                .replace(parsed)
                .is_some_and(|existing| existing != parsed)
            {
                return Err(HttpRequestReadError::bad_request(
                    "存在冲突的 Content-Length 请求头",
                ));
            }
        } else if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            let value = value.trim().to_ascii_lowercase();
            if let Some(existing) = transfer_encoding.as_mut() {
                existing.push(',');
                existing.push_str(&value);
            } else {
                transfer_encoding = Some(value);
            }
        }
    }

    if transfer_encoding.is_some() && content_length.is_some() {
        return Err(HttpRequestReadError::bad_request(
            "Transfer-Encoding 与 Content-Length 不能同时使用",
        ));
    }
    match transfer_encoding.as_deref() {
        Some("chunked") => Ok(HttpBodyFraming::Chunked),
        Some(_) => Err(HttpRequestReadError::bad_request(
            "仅支持 Transfer-Encoding: chunked",
        )),
        None => Ok(content_length
            .map(HttpBodyFraming::ContentLength)
            .unwrap_or(HttpBodyFraming::Empty)),
    }
}

fn content_length_body(
    encoded: &[u8],
    content_length: usize,
) -> Result<Vec<u8>, HttpRequestReadError> {
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(HttpRequestReadError::payload_too_large());
    }
    if encoded.len() < content_length {
        return Err(HttpRequestReadError::bad_request("HTTP 请求体不完整"));
    }
    Ok(encoded[..content_length].to_vec())
}

fn decode_chunked_body(encoded: &[u8]) -> Result<ChunkedBody, HttpRequestReadError> {
    let mut decoded = Vec::new();
    let mut position = 0;
    loop {
        let Some(line_end_offset) = encoded[position..]
            .windows(2)
            .position(|window| window == b"\r\n")
        else {
            if encoded.len().saturating_sub(position) > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
            }
            return Ok(ChunkedBody::Incomplete);
        };
        if line_end_offset > MAX_HTTP_HEADER_BYTES {
            return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
        }
        let line_end = position + line_end_offset;
        let size_text = std::str::from_utf8(&encoded[position..line_end])
            .map_err(|_| HttpRequestReadError::bad_request("chunk size 不是有效 ASCII"))?;
        let size_token = size_text.split(';').next().unwrap_or_default().trim();
        let chunk_size = usize::from_str_radix(size_token, 16)
            .map_err(|_| HttpRequestReadError::bad_request("chunk size 无效"))?;
        position = line_end + 2;

        if chunk_size == 0 {
            loop {
                let Some(trailer_end_offset) = encoded[position..]
                    .windows(2)
                    .position(|window| window == b"\r\n")
                else {
                    if encoded.len().saturating_sub(line_end + 2) > MAX_HTTP_HEADER_BYTES {
                        return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                    }
                    return Ok(ChunkedBody::Incomplete);
                };
                if position + trailer_end_offset - (line_end + 2) > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                }
                if trailer_end_offset == 0 {
                    return Ok(ChunkedBody::Complete(decoded));
                }
                position += trailer_end_offset + 2;
            }
        }
        if decoded.len().saturating_add(chunk_size) > MAX_HTTP_BODY_BYTES {
            return Err(HttpRequestReadError::payload_too_large());
        }
        let chunk_end = position
            .checked_add(chunk_size)
            .ok_or_else(HttpRequestReadError::payload_too_large)?;
        if encoded.len() < chunk_end + 2 {
            return Ok(ChunkedBody::Incomplete);
        }
        if &encoded[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err(HttpRequestReadError::bad_request("chunk 数据后缺少 CRLF"));
        }
        decoded.extend_from_slice(&encoded[position..chunk_end]);
        position = chunk_end + 2;
    }
}

#[cfg(test)]
fn scan_chunked_body(encoded: &[u8]) -> Result<ChunkedBodyScan, HttpRequestReadError> {
    ChunkedScanState::default().advance(encoded)
}

fn header_value_from_headers(headers: &str, header_name: &str) -> Option<String> {
    headers
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(header_name)
                .then(|| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
}

fn sanitize_diagnostic_event(event: &str) -> String {
    let sanitized = event
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "event".to_string()
    } else {
        sanitized
    }
}

pub fn build_codex_arguments(debug_port: u16, extra_args: &[String]) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={debug_port}"),
        format!("--remote-allow-origins=http://127.0.0.1:{debug_port}"),
    ];
    args.extend(normalize_codex_extra_args(extra_args));
    args
}

pub fn build_codex_arguments_for_settings(
    debug_port: u16,
    settings: &BackendSettings,
) -> Vec<String> {
    build_codex_arguments(
        debug_port,
        &codex_extra_args_for_launch(settings, &settings.codex_extra_args),
    )
}

fn codex_extra_args_for_launch(settings: &BackendSettings, extra_args: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    if settings.codex_app_fast_startup && !has_host_resolver_rules(extra_args) {
        args.push(statsig_fast_fail_host_resolver_rule());
    }
    args.extend(normalize_codex_extra_args(extra_args));
    args
}

fn has_host_resolver_rules(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg.trim().starts_with("--host-resolver-rules"))
}

fn statsig_fast_fail_host_resolver_rule() -> String {
    [
        "--host-resolver-rules=MAP ab.chatgpt.com 127.0.0.1",
        "MAP featureassets.org 127.0.0.1",
        "MAP prodregistryv2.org 127.0.0.1",
        "MAP api.statsigcdn.com 127.0.0.1",
        "MAP statsigapi.net 127.0.0.1",
        "MAP cloudflare-dns.com 127.0.0.1",
    ]
    .join(",")
}

pub fn build_codex_arguments_with_native_menu_inspector(
    debug_port: u16,
    inspector_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let mut args = build_codex_arguments(debug_port, &[]);
    if inspector_port != 0 {
        args.push(format!("--inspect=127.0.0.1:{inspector_port}"));
    }
    args.extend(normalize_codex_extra_args(extra_args));
    args
}

pub fn build_codex_command(app_dir: &Path, debug_port: u16, extra_args: &[String]) -> Vec<String> {
    let mut command = vec![
        crate::app_paths::build_codex_executable(app_dir)
            .to_string_lossy()
            .to_string(),
    ];
    command.extend(build_codex_arguments(debug_port, extra_args));
    command
}

pub fn build_codex_command_with_native_menu_inspector(
    app_dir: &Path,
    debug_port: u16,
    inspector_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let mut command = vec![
        crate::app_paths::build_codex_executable(app_dir)
            .to_string_lossy()
            .to_string(),
    ];
    command.extend(build_codex_arguments_with_native_menu_inspector(
        debug_port,
        inspector_port,
        extra_args,
    ));
    command
}

pub fn build_packaged_activation(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
) -> Option<CodexLaunch> {
    Some(CodexLaunch::PackagedActivation {
        app_user_model_id: crate::app_paths::packaged_app_user_model_id(app_dir)?,
        arguments: command_line_arguments(&build_codex_arguments(debug_port, extra_args)),
        process_id: None,
    })
}

pub fn build_packaged_activation_with_native_menu_inspector(
    app_dir: &Path,
    debug_port: u16,
    inspector_port: u16,
    extra_args: &[String],
) -> Option<CodexLaunch> {
    Some(CodexLaunch::PackagedActivation {
        app_user_model_id: crate::app_paths::packaged_app_user_model_id(app_dir)?,
        arguments: command_line_arguments(&build_codex_arguments_with_native_menu_inspector(
            debug_port,
            inspector_port,
            extra_args,
        )),
        process_id: None,
    })
}

async fn retry_injection(debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
    let mut last_error = None;
    for _ in 0..20 {
        match try_inject(debug_port, helper_port).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Codex injection failed")))
}

async fn wait_for_codex_page(debug_port: u16) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async move {
        loop {
            if crate::cdp::list_targets(debug_port)
                .await
                .and_then(|targets| {
                    crate::cdp::pick_injectable_codex_page_target(&targets).map(|_| ())
                })
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Codex page did not become ready within 30 seconds"))
}

pub async fn check_and_reinject_bridge(debug_port: u16, helper_port: u16) -> bool {
    check_and_reinject_bridge_inner(debug_port, helper_port, false).await
}

pub fn browser_identity_changed(previous: Option<&str>, current: &str) -> bool {
    previous.is_some_and(|previous| previous != current)
}

async fn check_and_reinject_bridge_inner(
    debug_port: u16,
    helper_port: u16,
    browser_identity_changed: bool,
) -> bool {
    let healthy = if browser_identity_changed {
        false
    } else {
        match bridge_health_ok(debug_port).await {
            Ok(healthy) => healthy,
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.health_check_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port,
                        "message": error.to_string()
                    }),
                );
                false
            }
        }
    };
    if healthy {
        return false;
    }

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.reinject_start",
        serde_json::json!({
            "debug_port": debug_port,
            "helper_port": helper_port,
            "browser_identity_changed": browser_identity_changed
        }),
    );
    match retry_injection(debug_port, helper_port).await {
        Ok(()) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.reinject_ok",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port
                }),
            );
            true
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.reinject_failed",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port,
                    "message": error.to_string()
                }),
            );
            false
        }
    }
}

async fn bridge_health_ok(debug_port: u16) -> anyhow::Result<bool> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let result = crate::bridge::evaluate_script_with_await_promise(
        websocket_url,
        crate::bridge::bridge_health_check_script(),
        true,
    )
    .await?;
    Ok(runtime_evaluate_result_is_true(&result))
}

fn runtime_evaluate_result_is_true(result: &Value) -> bool {
    result
        .get("result")
        .and_then(|result| result.get("result"))
        .and_then(|result| result.get("value"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

async fn try_inject(debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let settings = SettingsStore::default().load().unwrap_or_default();
    let script = crate::assets::injection_script_with_settings(helper_port, &settings);
    let ctx = crate::routes::BridgeContext::core(Arc::new(crate::routes::CoreRuntimeService::new(
        debug_port,
        StatusStore::default(),
    )));
    crate::bridge::install_bridge(
        websocket_url,
        crate::bridge::BRIDGE_BINDING_NAME,
        Arc::new(move |path, payload| {
            let ctx = ctx.clone();
            Box::pin(
                async move { Ok(crate::routes::handle_bridge_request(ctx, &path, payload).await) },
            )
        }),
        &[script],
    )
    .await
}

async fn confirmed_pet_overlay_targets(
    debug_port: u16,
) -> anyhow::Result<Vec<crate::cdp::CdpTarget>> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let mut confirmed = Vec::new();
    for target in targets
        .into_iter()
        .filter(crate::cdp::is_avatar_overlay_page_target)
    {
        let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
            continue;
        };
        if pet_overlay_supports_v2_cursor(websocket_url).await {
            confirmed.push(target);
        }
    }
    Ok(confirmed)
}

async fn pet_overlay_supports_v2_cursor(websocket_url: &str) -> bool {
    crate::bridge::evaluate_script_with_await_promise(
        websocket_url,
        &crate::assets::pet_real_mouse_capability_probe_script(),
        true,
    )
    .await
    .as_ref()
    .is_ok_and(runtime_evaluate_result_is_true)
}

async fn sync_pet_real_mouse_overlay(debug_port: u16, _helper_port: u16) -> anyhow::Result<()> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let enabled = settings.enhancements_enabled && settings.codex_app_pet_real_mouse_look;
    let targets = crate::cdp::list_targets(debug_port).await?;
    for target in targets
        .iter()
        .filter(|target| crate::cdp::is_avatar_overlay_page_target(target))
    {
        let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
            continue;
        };
        let supports_v2 = enabled && pet_overlay_supports_v2_cursor(websocket_url).await;
        let script = if supports_v2 {
            crate::assets::pet_real_mouse_script()
        } else {
            crate::assets::pet_real_mouse_stop_script()
        };
        crate::bridge::evaluate_script(websocket_url, script)
            .await
            .with_context(|| {
                format!(
                    "failed to evaluate pet overlay script in target {} ({})",
                    target.id, target.url
                )
            })?;
    }
    Ok(())
}

#[cfg(windows)]
async fn run_pet_real_mouse_cursor_driver(debug_port: u16) {
    loop {
        let settings = SettingsStore::default().load().unwrap_or_default();
        if !settings.enhancements_enabled || !settings.codex_app_pet_real_mouse_look {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }

        let targets = confirmed_pet_overlay_targets(debug_port)
            .await
            .unwrap_or_default();
        if targets.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let mut drivers = tokio::task::JoinSet::new();
        for target in targets.iter().cloned() {
            drivers.spawn(run_pet_real_mouse_target_driver(debug_port, target));
        }
        if let Some(result) = drivers.join_next().await {
            if let Err(error) = result {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_cursor_driver_join_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "message": error.to_string()
                    }),
                );
            }
        }
        for target in &targets {
            if let Some(websocket_url) = target.web_socket_debugger_url.as_deref() {
                let _ = crate::bridge::evaluate_script(
                    websocket_url,
                    crate::assets::pet_real_mouse_stop_script(),
                )
                .await;
            }
        }
        drivers.abort_all();
        while drivers.join_next().await.is_some() {}
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[cfg(windows)]
async fn run_pet_real_mouse_target_driver(debug_port: u16, target: crate::cdp::CdpTarget) {
    let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
        return;
    };
    if let Err(error) =
        crate::bridge::evaluate_script(websocket_url, crate::assets::pet_real_mouse_script()).await
    {
        record_pet_cursor_driver_failure(debug_port, &target, error);
        return;
    }

    let mut ticks_until_settings_check = 10_u8;
    let result = crate::bridge::run_periodic_evaluations(
        websocket_url,
        std::time::Duration::from_millis(100),
        || {
            if ticks_until_settings_check == 0 {
                let settings = SettingsStore::default().load().unwrap_or_default();
                if !settings.enhancements_enabled || !settings.codex_app_pet_real_mouse_look {
                    return Ok(None);
                }
                ticks_until_settings_check = 10;
            }
            ticks_until_settings_check -= 1;
            let (x, y) = windows_logical_cursor_position()?;
            Ok(Some(crate::assets::pet_real_mouse_update_script(x, y)))
        },
    )
    .await;

    let _ =
        crate::bridge::evaluate_script(websocket_url, crate::assets::pet_real_mouse_stop_script())
            .await;
    match result {
        Ok(()) => {
            PET_CURSOR_DRIVER_FAILED.store(false, Ordering::Relaxed);
        }
        Err(error) => record_pet_cursor_driver_failure(debug_port, &target, error),
    }
}

#[cfg(windows)]
fn record_pet_cursor_driver_failure(
    debug_port: u16,
    target: &crate::cdp::CdpTarget,
    error: anyhow::Error,
) {
    if !PET_CURSOR_DRIVER_FAILED.swap(true, Ordering::Relaxed) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "pet.real_mouse_cursor_driver_disconnected",
            serde_json::json!({
                "debug_port": debug_port,
                "target_id": target.id,
                "target_url": target.url,
                "message": format!("{error:#}")
            }),
        );
    }
}

fn record_pet_overlay_sync_result(debug_port: u16, helper_port: u16, result: anyhow::Result<()>) {
    match result {
        Ok(()) => {
            if PET_OVERLAY_SYNC_FAILED.swap(false, Ordering::Relaxed) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_overlay_sync_recovered",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port
                    }),
                );
            }
        }
        Err(error) => {
            if !PET_OVERLAY_SYNC_FAILED.swap(true, Ordering::Relaxed) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_overlay_sync_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port,
                        "message": format!("{error:#}")
                    }),
                );
            }
        }
    }
}

pub fn build_macos_open_command(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let mut command = vec![
        "open".to_string(),
        "-W".to_string(),
        "-a".to_string(),
        app_dir.to_string_lossy().to_string(),
        "--args".to_string(),
    ];
    command.extend(build_codex_arguments(debug_port, extra_args));
    command
}

pub fn build_macos_open_command_with_native_menu_inspector(
    app_dir: &Path,
    debug_port: u16,
    inspector_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let mut command = vec![
        "open".to_string(),
        "-W".to_string(),
        "-a".to_string(),
        app_dir.to_string_lossy().to_string(),
        "--args".to_string(),
    ];
    command.extend(build_codex_arguments_with_native_menu_inspector(
        debug_port,
        inspector_port,
        extra_args,
    ));
    command
}

pub fn build_macos_cleanup_command(
    app_dir: &Path,
    policy: MacosCleanupPolicy,
) -> Option<Vec<String>> {
    if policy == MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning {
        return None;
    }
    let app_name = app_dir
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Codex");
    Some(vec![
        "osascript".to_string(),
        "-e".to_string(),
        format!(
            r#"tell application "{}" to quit"#,
            app_name.replace('"', "\\\"")
        ),
    ])
}

async fn terminate_tracked_child(child: &mut Child) -> anyhow::Result<()> {
    match child.kill().await {
        Ok(()) => Ok(()),
        Err(error) => match child.try_wait()? {
            Some(_) => Ok(()),
            None => Err(error).context("failed to terminate tracked Codex process"),
        },
    }
}

async fn wait_for_tracked_child_exit(child: &mut Child) -> anyhow::Result<()> {
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .context("timed out waiting for tracked Codex process to exit")?
        .context("failed to wait for tracked Codex process")?;
    if !status.success() {
        anyhow::bail!("tracked Codex process exited with status {status}");
    }
    Ok(())
}

async fn wait_for_tracked_child_exit_without_timeout(child: &mut Child) -> anyhow::Result<()> {
    let status = child
        .wait()
        .await
        .context("failed to wait for tracked Codex process")?;
    if !status.success() {
        anyhow::bail!("tracked Codex process exited with status {status}");
    }
    Ok(())
}

async fn wait_for_tracked_child_exit_confirmed(child: &mut Child) -> anyhow::Result<()> {
    loop {
        match child.wait().await {
            Ok(_) => return Ok(()),
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
}

async fn run_macos_cleanup_command(
    app_dir: &Path,
    policy: MacosCleanupPolicy,
) -> anyhow::Result<()> {
    let Some(command) = build_macos_cleanup_command(app_dir, policy) else {
        return Ok(());
    };
    let Some(executable) = command.first() else {
        return Ok(());
    };
    let mut quit = Command::new(executable);
    quit.args(&command[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), quit.status())
        .await
        .context("timed out requesting macOS app quit")?
        .with_context(|| format!("failed to request macOS app quit for {}", app_dir.display()))?;
    if !status.success() {
        anyhow::bail!(
            "macOS app quit request failed for {} with status {status}",
            app_dir.display()
        );
    }
    Ok(())
}

fn macos_app_dir_from_open_command(command: &[String]) -> Option<PathBuf> {
    let app_index = command.iter().position(|part| part == "-a")?;
    command.get(app_index + 1).map(PathBuf::from)
}

async fn is_macos_app_running(app_dir: &Path) -> anyhow::Result<bool> {
    if !cfg!(target_os = "macos") {
        return Ok(false);
    }
    let app_name = app_dir
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Codex");
    let script = format!(
        r#"application "{}" is running"#,
        app_name.replace('"', "\\\"")
    );
    let mut query = Command::new("osascript");
    query
        .arg("-e")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(2), query.output())
        .await
        .context("timed out querying macOS app state")?
        .with_context(|| format!("failed to query macOS app state for {}", app_dir.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "macOS app state query failed for {} with status {}",
            app_dir.display(),
            output.status
        );
    }
    let value = String::from_utf8_lossy(&output.stdout);
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => anyhow::bail!(
            "macOS app state query returned an invalid value for {}: {other}",
            app_dir.display()
        ),
    }
}

async fn wait_for_macos_app_exit(app_dir: &Path) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while is_macos_app_running(app_dir).await? {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for macOS app exit: {}",
                app_dir.display()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(())
}

pub async fn wait_for_confirmed_macos_app_exit_with<F, Fut, E>(
    mut is_running: F,
    retry_interval: std::time::Duration,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, E>>,
{
    loop {
        if matches!(is_running().await, Ok(false)) {
            return;
        }
        tokio::time::sleep(retry_interval).await;
    }
}

pub async fn wait_for_macos_exit_after_bounded_failures_with<
    WaiterFuture,
    Query,
    QueryFuture,
    QueryError,
>(
    waiter: Option<WaiterFuture>,
    is_running: Query,
    retry_interval: std::time::Duration,
) -> Option<anyhow::Error>
where
    WaiterFuture: std::future::Future<Output = anyhow::Result<()>>,
    Query: FnMut() -> QueryFuture,
    QueryFuture: std::future::Future<Output = Result<bool, QueryError>>,
{
    let waiter_error = if let Some(waiter) = waiter {
        match waiter.await {
            Ok(()) => return None,
            Err(error) => Some(error),
        }
    } else {
        None
    };
    wait_for_confirmed_macos_app_exit_with(is_running, retry_interval).await;
    waiter_error
}

#[cfg_attr(not(windows), allow(dead_code))]
fn post_launch_guard_artifacts_ready(
    artifacts: &crate::computer_use_guard::GuardArtifacts,
) -> bool {
    artifacts.notify_exe.is_some()
        && artifacts.marketplace_path.is_some()
        && (!artifacts.runtime_exports_needed || artifacts.sky_package_json.is_some())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn should_stop_post_launch_computer_use_guard(
    stable_unchanged_attempts: usize,
    artifacts: &crate::computer_use_guard::GuardArtifacts,
) -> bool {
    stable_unchanged_attempts >= POST_LAUNCH_COMPUTER_USE_GUARD_STABLE_ATTEMPTS
        && post_launch_guard_artifacts_ready(artifacts)
}

#[cfg(windows)]
async fn run_post_launch_computer_use_guard(
    home: PathBuf,
    mut artifacts: Option<crate::computer_use_guard::GuardArtifacts>,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
) {
    let mut previous_delay = 0_u64;
    let mut stable_unchanged_attempts = 0_usize;
    for (index, delay) in POST_LAUNCH_COMPUTER_USE_GUARD_SECONDS
        .iter()
        .copied()
        .enumerate()
    {
        let wait_seconds = delay.saturating_sub(previous_delay);
        previous_delay = delay;
        if wait_seconds > 0 {
            tokio::select! {
                _ = &mut *shutdown_rx => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait_seconds)) => {}
            }
        }
        let attempt = index + 1;
        let resolved_artifacts = match artifacts.take() {
            Some(artifacts) => artifacts,
            None => match crate::computer_use_guard::resolve_computer_use_guard_artifacts(&home) {
                Ok(resolved) => resolved,
                Err(error) => {
                    stable_unchanged_attempts = 0;
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "computer_use_guard.post_launch_failed",
                        serde_json::json!({
                            "attempt": attempt,
                            "delay_seconds": delay,
                            "phase": "resolve_artifacts",
                            "message": error.to_string()
                        }),
                    );
                    continue;
                }
            },
        };
        let artifacts_ready = post_launch_guard_artifacts_ready(&resolved_artifacts);
        artifacts = artifacts_ready.then_some(resolved_artifacts.clone());
        match crate::computer_use_guard::ensure_computer_use_config_with_artifacts(
            &home,
            &resolved_artifacts,
        ) {
            Ok(result) => {
                if !result.changed && artifacts_ready {
                    stable_unchanged_attempts += 1;
                } else {
                    stable_unchanged_attempts = 0;
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "computer_use_guard.post_launch_ok",
                    serde_json::json!({
                        "attempt": attempt,
                        "delay_seconds": delay,
                        "changed": result.changed,
                        "stable_unchanged_attempts": stable_unchanged_attempts,
                        "notify_exe": result
                            .notify_exe
                            .map(|path| path.to_string_lossy().to_string())
                    }),
                );
                if should_stop_post_launch_computer_use_guard(
                    stable_unchanged_attempts,
                    &resolved_artifacts,
                ) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "computer_use_guard.post_launch_stable_stop",
                        serde_json::json!({
                            "attempt": attempt,
                            "delay_seconds": delay,
                            "stable_unchanged_attempts": stable_unchanged_attempts
                        }),
                    );
                    return;
                }
            }
            Err(error) => {
                stable_unchanged_attempts = 0;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "computer_use_guard.post_launch_failed",
                    serde_json::json!({
                        "attempt": attempt,
                        "delay_seconds": delay,
                        "message": error.to_string()
                    }),
                );
            }
        }
    }
}

#[cfg(windows)]
async fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || wait_for_windows_process_id_blocking(process_id))
        .await
        .context("Windows process wait task failed")?
}

#[cfg(windows)]
async fn wait_for_windows_process_exit_confirmed(process_id: u32) {
    loop {
        if wait_for_windows_process_id(process_id).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(windows)]
fn windows_process_parent_map() -> anyhow::Result<HashMap<u32, u32>> {
    let processes = crate::windows_integration::try_enumerate_processes()?;
    Ok(processes
        .iter()
        .map(|process| (process.process_id, process.parent_process_id))
        .collect())
}

#[cfg(windows)]
fn try_capture_windows_descendant_handles(
    root: &WindowsProcessHandle,
) -> anyhow::Result<Vec<WindowsProcessHandle>> {
    if windows_process_handle_has_exited(root)? {
        anyhow::bail!("Windows root process exited before descendant identity capture");
    }
    let root_identity = root.identity();
    let first_parents = windows_process_parent_map()?;
    let mut descendant_ids = first_parents
        .keys()
        .copied()
        .filter(|process_id| {
            *process_id != root_identity.process_id()
                && process_descends_from(*process_id, root_identity.process_id(), &first_parents)
        })
        .collect::<Vec<_>>();
    descendant_ids.sort_unstable();

    let mut handles = HashMap::new();
    for process_id in descendant_ids {
        let handle = open_windows_process_handle(process_id, false)?.with_context(|| {
            format!("Windows descendant process {process_id} exited before handle acquisition")
        })?;
        handles.insert(process_id, handle);
    }

    let second_parents = windows_process_parent_map()?;
    if windows_process_handle_has_exited(root)? {
        anyhow::bail!("Windows root process exited during descendant identity capture");
    }
    let opened_identities = handles
        .iter()
        .map(|(process_id, handle)| (*process_id, handle.identity()))
        .collect::<HashMap<_, _>>();
    let verified = validate_windows_descendant_identities(
        root_identity,
        &first_parents,
        &second_parents,
        &opened_identities,
    )?;
    let mut verified_handles = Vec::with_capacity(verified.len());
    for identity in verified {
        let handle = handles
            .remove(&identity.process_id())
            .context("verified Windows descendant handle is unavailable")?;
        if windows_process_handle_has_exited(&handle)? {
            anyhow::bail!(
                "Windows descendant process {} exited during identity validation",
                identity.process_id()
            );
        }
        verified_handles.push(handle);
    }
    if windows_process_handle_has_exited(root)? {
        anyhow::bail!("Windows root process exited after descendant identity validation");
    }
    Ok(verified_handles)
}

fn process_descends_from(
    process_id: u32,
    root_process_id: u32,
    parents: &HashMap<u32, u32>,
) -> bool {
    let mut current = process_id;
    for _ in 0..=parents.len() {
        let Some(parent) = parents.get(&current).copied() else {
            return false;
        };
        if parent == root_process_id {
            return true;
        }
        if parent == 0 || parent == current {
            return false;
        }
        current = parent;
    }
    false
}

fn validate_windows_descendant_identities(
    root: WindowsProcessIdentity,
    first_parents: &HashMap<u32, u32>,
    second_parents: &HashMap<u32, u32>,
    opened: &HashMap<u32, WindowsProcessIdentity>,
) -> anyhow::Result<Vec<WindowsProcessIdentity>> {
    let descendant_ids = |parents: &HashMap<u32, u32>| {
        let mut process_ids = parents
            .keys()
            .copied()
            .filter(|process_id| {
                *process_id != root.process_id()
                    && process_descends_from(*process_id, root.process_id(), parents)
            })
            .collect::<Vec<_>>();
        process_ids.sort_unstable();
        process_ids
    };
    let first_descendants = descendant_ids(first_parents);
    let second_descendants = descendant_ids(second_parents);
    if first_descendants != second_descendants {
        anyhow::bail!("Windows descendant process tree changed during identity capture");
    }

    let mut identities = Vec::with_capacity(second_descendants.len());
    for process_id in second_descendants {
        let identity = opened.get(&process_id).copied().with_context(|| {
            format!("Windows descendant process {process_id} has no stable handle identity")
        })?;
        let first_parent = first_parents.get(&process_id).copied();
        let second_parent = second_parents.get(&process_id).copied();
        if first_parent != second_parent {
            anyhow::bail!(
                "Windows descendant process {process_id} parent changed during identity capture"
            );
        }
        let parent_process_id =
            second_parent.context("Windows descendant parent is unavailable")?;
        let parent_identity = if parent_process_id == root.process_id() {
            root
        } else {
            opened.get(&parent_process_id).copied().with_context(|| {
                format!(
                    "Windows descendant parent process {parent_process_id} has no stable handle identity"
                )
            })?
        };
        if parent_identity.creation_time() >= identity.creation_time() {
            anyhow::bail!(
                "Windows descendant process {process_id} has an invalid parent/child creation order"
            );
        }
        identities.push(identity);
    }
    identities.sort_unstable_by_key(|identity| identity.process_id());
    Ok(identities)
}

#[cfg(windows)]
async fn wait_for_verified_windows_descendant_handles(
    root: &WindowsProcessHandle,
) -> anyhow::Result<Vec<WindowsProcessHandle>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_error = None;
    loop {
        match try_capture_windows_descendant_handles(root) {
            Ok(handles) => return Ok(handles),
            Err(error) => last_error = Some(error),
        }
        if windows_process_handle_has_exited(root)? || tokio::time::Instant::now() >= deadline {
            return Err(last_error.unwrap_or_else(|| {
                anyhow::anyhow!("Windows descendant identity could not be confirmed")
            }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(windows)]
async fn cleanup_owned_windows_process(root: WindowsProcessHandle) -> anyhow::Result<()> {
    let descendants = wait_for_verified_windows_descendant_handles(&root).await;
    cleanup_confirmed_process_tree_with(
        root,
        descendants,
        terminate_windows_process_handle,
        |handle| async move {
            wait_for_windows_process_handle_confirmed(handle).await;
        },
        |handle| async move {
            wait_for_windows_process_handle_confirmed(handle).await;
        },
    )
    .await
}

#[cfg(target_os = "linux")]
async fn terminate_linux_process_group_and_wait(process_group: u32) -> anyhow::Result<()> {
    if linux_process_group_members(process_group).is_empty() {
        return Ok(());
    }
    let mut cleanup_errors = Vec::new();
    record_cleanup_result(
        &mut cleanup_errors,
        "send TERM to Linux Codex process group",
        send_linux_process_group_signal(process_group, "-TERM").await,
    );
    let remaining = wait_for_linux_process_group_exit(process_group).await;
    if remaining.is_empty() {
        return finish_cleanup(cleanup_errors);
    }
    record_cleanup_result(
        &mut cleanup_errors,
        "send KILL to Linux Codex process group",
        send_linux_process_group_signal(process_group, "-KILL").await,
    );
    let remaining = wait_for_linux_process_group_exit(process_group).await;
    if !remaining.is_empty() {
        cleanup_errors.push(format!(
            "Linux Codex process group {process_group} did not exit after termination: {}",
            remaining
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    finish_cleanup(cleanup_errors)
}

#[cfg(target_os = "linux")]
async fn send_linux_process_group_signal(process_group: u32, signal: &str) -> anyhow::Result<()> {
    Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{process_group}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .with_context(|| {
            format!("failed to send {signal} to Linux process group {process_group}")
        })?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn wait_for_linux_process_group_exit(process_group: u32) -> Vec<u32> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let remaining = linux_process_group_members(process_group);
        if remaining.is_empty() || tokio::time::Instant::now() >= deadline {
            return remaining;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(target_os = "linux")]
fn linux_process_group_members(process_group: u32) -> Vec<u32> {
    let mut members = std::fs::read_dir("/proc")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let process_id = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            (linux_process_group_from_stat(&stat) == Some(process_group)).then_some(process_id)
        })
        .collect::<Vec<_>>();
    members.sort_unstable();
    members
}

#[cfg(target_os = "linux")]
fn linux_process_group_from_stat(stat: &str) -> Option<u32> {
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

#[cfg(windows)]
fn wait_for_windows_process_id_blocking(process_id: u32) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_FAILED};
    use windows::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };

    unsafe {
        let handle = match OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        ) {
            Ok(handle) => handle,
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to open Windows process id {process_id}"));
            }
        };
        let wait_result = WaitForSingleObject(handle, INFINITE);
        let _ = CloseHandle(handle);
        if wait_result == WAIT_FAILED {
            anyhow::bail!("failed to wait for Windows process id {process_id}");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_windows_process_handle(
    process_id: u32,
    allow_termination: bool,
) -> anyhow::Result<Option<WindowsProcessHandle>> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    unsafe {
        let access = if allow_termination {
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION
        } else {
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION
        };
        let handle = match OpenProcess(access, false, process_id) {
            Ok(handle) => handle,
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to acquire Windows process id {process_id}"));
            }
        };

        let identity = (|| -> anyhow::Result<WindowsProcessIdentity> {
            let observed_process_id = GetProcessId(handle);
            if observed_process_id != process_id {
                anyhow::bail!(
                    "Windows process handle identity mismatch: expected {process_id}, observed {observed_process_id}"
                );
            }
            let mut creation_time = FILETIME::default();
            let mut exit_time = FILETIME::default();
            let mut kernel_time = FILETIME::default();
            let mut user_time = FILETIME::default();
            GetProcessTimes(
                handle,
                &mut creation_time,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            )
            .with_context(|| format!("failed to query Windows process id {process_id} times"))?;
            Ok(WindowsProcessIdentity::new(
                process_id,
                filetime_value(creation_time),
            ))
        })();
        let identity = match identity {
            Ok(identity) => identity,
            Err(error) => {
                let _ = CloseHandle(handle);
                return Err(error);
            }
        };
        Ok(Some(WindowsProcessHandle(Arc::new(
            WindowsProcessHandleInner {
                raw_handle: handle.0 as usize,
                identity,
            },
        ))))
    }
}

#[cfg(windows)]
fn filetime_value(value: windows::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

#[cfg(windows)]
fn current_windows_filetime() -> anyhow::Result<u64> {
    const WINDOWS_TO_UNIX_EPOCH_SECONDS: u64 = 11_644_473_600;
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    Ok(
        (elapsed.as_secs() + WINDOWS_TO_UNIX_EPOCH_SECONDS) * 10_000_000
            + u64::from(elapsed.subsec_nanos()) / 100,
    )
}

#[cfg(windows)]
fn windows_process_handle_has_exited(handle: &WindowsProcessHandle) -> anyhow::Result<bool> {
    use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::WaitForSingleObject;

    let wait_result = unsafe { WaitForSingleObject(handle.raw_handle(), 0) };
    match wait_result {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => anyhow::bail!(
            "failed to query Windows process id {} state",
            handle.identity().process_id()
        ),
        other => anyhow::bail!(
            "unexpected wait result {other:?} for Windows process id {}",
            handle.identity().process_id()
        ),
    }
}

#[cfg(windows)]
fn wait_for_windows_process_handle_blocking(handle: &WindowsProcessHandle) -> anyhow::Result<()> {
    use windows::Win32::Foundation::WAIT_FAILED;
    use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};

    let wait_result = unsafe { WaitForSingleObject(handle.raw_handle(), INFINITE) };
    if wait_result == WAIT_FAILED {
        anyhow::bail!(
            "failed to wait for Windows process id {}",
            handle.identity().process_id()
        );
    }
    Ok(())
}

#[cfg(windows)]
async fn wait_for_windows_process_handle(handle: WindowsProcessHandle) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || wait_for_windows_process_handle_blocking(&handle))
        .await
        .context("Windows process handle wait task failed")?
}

#[cfg(windows)]
async fn wait_for_windows_process_handle_confirmed(handle: WindowsProcessHandle) {
    loop {
        if wait_for_windows_process_handle(handle.clone())
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(windows)]
fn terminate_windows_process_handle_blocking(handle: &WindowsProcessHandle) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{INFINITE, TerminateProcess, WaitForSingleObject};

    unsafe {
        let initial_wait_result = WaitForSingleObject(handle.raw_handle(), 0);
        if initial_wait_result == WAIT_OBJECT_0 {
            return Ok(());
        }
        if initial_wait_result == WAIT_FAILED {
            anyhow::bail!(
                "failed to query Windows process id {} before termination",
                handle.identity().process_id()
            );
        }
        let terminate_result = TerminateProcess(handle.raw_handle(), 1);
        let wait_result = terminate_result
            .is_ok()
            .then(|| WaitForSingleObject(handle.raw_handle(), INFINITE));
        terminate_result.with_context(|| {
            format!(
                "failed to terminate Windows process id {}",
                handle.identity().process_id()
            )
        })?;
        if wait_result == Some(WAIT_FAILED) {
            anyhow::bail!(
                "failed to wait for terminated Windows process id {}",
                handle.identity().process_id()
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn terminate_windows_process_handle(handle: WindowsProcessHandle) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || terminate_windows_process_handle_blocking(&handle))
        .await
        .context("Windows process handle termination task failed")?
}

#[cfg(not(windows))]
async fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    anyhow::bail!("cannot wait for Windows process id {process_id} on this platform")
}

fn launch_status(
    status: &str,
    message: &str,
    debug_port: u16,
    helper_port: u16,
    app_dir: &Path,
) -> LaunchStatus {
    LaunchStatus {
        status: status.to_string(),
        message: message.to_string(),
        started_at_ms: now_ms(),
        debug_port: Some(debug_port),
        helper_port: Some(helper_port),
        codex_app: Some(app_dir.to_string_lossy().to_string()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn command_line_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.bytes().any(|byte| matches!(byte, b' ' | b'\t' | b'"')) {
        return arg.to_string();
    }
    let mut output = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                output.push_str(&"\\".repeat(backslashes * 2 + 1));
                output.push('"');
                backslashes = 0;
            }
            _ => {
                output.push_str(&"\\".repeat(backslashes));
                output.push(ch);
                backslashes = 0;
            }
        }
    }
    output.push_str(&"\\".repeat(backslashes * 2));
    output.push('"');
    output
}

#[cfg(not(windows))]
pub async fn activate_packaged_app(
    _app_user_model_id: &str,
    _arguments: &str,
) -> anyhow::Result<u32> {
    anyhow::bail!("Packaged app activation is only supported on Windows")
}

fn capture_packaged_activation_with<T, Activate, Capture>(
    activate: Activate,
    capture: Capture,
) -> anyhow::Result<(u32, T)>
where
    Activate: FnOnce() -> anyhow::Result<u32>,
    Capture: FnOnce(u32) -> T,
{
    let process_id = activate()?;
    Ok((process_id, capture(process_id)))
}

#[cfg(windows)]
fn capture_windows_packaged_process_handle(
    process_id: u32,
) -> anyhow::Result<Option<WindowsProcessHandle>> {
    let activation_completed_at = current_windows_filetime()?;
    open_windows_process_handle(process_id, false).and_then(|handle| {
        if let Some(handle) = &handle
            && handle.identity().creation_time() > activation_completed_at
        {
            anyhow::bail!(
                "Windows activation process id {process_id} was reused before handle acquisition"
            );
        }
        Ok(handle)
    })
}

#[cfg(windows)]
async fn activate_packaged_app_with_process_handle(
    app_user_model_id: &str,
    arguments: &str,
) -> anyhow::Result<(u32, anyhow::Result<Option<WindowsProcessHandle>>)> {
    let app_user_model_id = app_user_model_id.to_string();
    let arguments = arguments.to_string();
    tokio::task::spawn_blocking(move || {
        activate_packaged_app_blocking_with(
            &app_user_model_id,
            &arguments,
            capture_windows_packaged_process_handle,
        )
    })
    .await
    .context("packaged app activation identity task failed")?
}

#[cfg(windows)]
pub async fn activate_packaged_app(
    app_user_model_id: &str,
    arguments: &str,
) -> anyhow::Result<u32> {
    let app_user_model_id = app_user_model_id.to_string();
    let arguments = arguments.to_string();
    tokio::task::spawn_blocking(move || {
        activate_packaged_app_blocking(&app_user_model_id, &arguments)
    })
    .await
    .context("packaged app activation task failed")?
}

#[cfg(windows)]
fn activate_packaged_app_blocking(app_user_model_id: &str, arguments: &str) -> anyhow::Result<u32> {
    activate_packaged_app_blocking_with(app_user_model_id, arguments, |_| ())
        .map(|(process_id, ())| process_id)
}

#[cfg(windows)]
fn activate_packaged_app_blocking_with<T, Capture>(
    app_user_model_id: &str,
    arguments: &str,
    capture: Capture,
) -> anyhow::Result<(u32, T)>
where
    Capture: FnOnce(u32) -> T,
{
    use windows::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager};
    use windows::core::HSTRING;

    unsafe {
        let coinit = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninitialize = coinit.is_ok();
        coinit.ok().or_else(|error| {
            const RPC_E_CHANGED_MODE: i32 = -2147417850;
            if error.code().0 == RPC_E_CHANGED_MODE {
                Ok(())
            } else {
                Err(error)
            }
        })?;

        let result = capture_packaged_activation_with(
            || {
                let manager: IApplicationActivationManager =
                    CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)?;
                manager
                    .ActivateApplication(
                        &HSTRING::from(app_user_model_id),
                        &HSTRING::from(arguments),
                        windows::Win32::UI::Shell::ACTIVATEOPTIONS(0),
                    )
                    .map_err(Into::into)
            },
            capture,
        );

        if should_uninitialize {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_launcher_tolerates_a_slow_codex_wrapper_restart() {
        let mut wait = ProcessExitObservation::new();

        for _ in 0..7 {
            assert!(!wait.observe(false));
        }
        assert!(!wait.observe(true));
    }

    #[test]
    fn packaged_activation_only_skips_an_exact_prelaunch_identity() {
        let existing = WindowsProcessIdentity::new(42, 100);
        let reused = WindowsProcessIdentity::new(42, 200);

        assert_eq!(
            packaged_process_cleanup_action(Some(&[existing]), Some(existing)),
            PackagedProcessCleanupAction::SkipExisting
        );
        assert_eq!(
            packaged_process_cleanup_action(Some(&[existing]), Some(reused)),
            PackagedProcessCleanupAction::WaitForExitWithoutTermination
        );
        assert_eq!(
            packaged_process_cleanup_action(None, Some(reused)),
            PackagedProcessCleanupAction::WaitForExitWithoutTermination
        );
    }

    #[test]
    fn packaged_activation_preserves_pid_when_identity_capture_fails() {
        let events = std::cell::RefCell::new(Vec::new());

        let (process_id, identity) = capture_packaged_activation_with(
            || {
                events.borrow_mut().push("activate");
                Ok(42)
            },
            |observed_process_id| -> anyhow::Result<Option<&'static str>> {
                events.borrow_mut().push("identify");
                assert_eq!(observed_process_id, 42);
                anyhow::bail!("identity unavailable")
            },
        )
        .expect("activation success must preserve the returned process id");

        assert_eq!(process_id, 42);
        assert_eq!(&*events.borrow(), &["activate", "identify"]);
        assert!(
            identity
                .unwrap_err()
                .to_string()
                .contains("identity unavailable")
        );
    }

    #[tokio::test]
    async fn existing_windows_packaged_cleanup_skips_waiting() {
        let waited = Arc::new(AtomicUsize::new(0));
        let wait_count = Arc::clone(&waited);

        cleanup_packaged_process_with(PackagedProcessCleanupAction::SkipExisting, move || {
            let wait_count = Arc::clone(&wait_count);
            async move {
                wait_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(waited.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unconfirmed_windows_packaged_cleanup_waits() {
        let waited = Arc::new(AtomicUsize::new(0));
        let wait_count = Arc::clone(&waited);

        let error = cleanup_packaged_process_with(
            PackagedProcessCleanupAction::WaitForExitWithoutTermination,
            move || {
                let wait_count = Arc::clone(&wait_count);
                async move {
                    wait_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("ownership was not confirmed"));
        assert_eq!(waited.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn packaged_baseline_collects_stable_identities_for_candidate_names() {
        let processes = [(41, "ChatGPT.exe"), (42, "helper.exe"), (43, "codex.EXE")];
        let queried = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&queried);

        let identities =
            packaged_activation_baseline_identities_with(&processes, move |process_id| {
                observed.lock().unwrap().push(process_id);
                Ok((process_id != 43).then(|| WindowsProcessIdentity::new(process_id, 100)))
            })
            .unwrap();

        assert_eq!(identities, vec![WindowsProcessIdentity::new(41, 100)]);
        assert_eq!(*queried.lock().unwrap(), vec![41, 43]);
    }

    #[test]
    fn packaged_baseline_identity_failure_invalidates_the_whole_baseline() {
        let error = packaged_activation_baseline_identities_with(
            &[(41, "ChatGPT.exe"), (43, "Codex.exe")],
            |process_id| {
                if process_id == 43 {
                    anyhow::bail!("identity unavailable");
                }
                Ok(Some(WindowsProcessIdentity::new(process_id, 100)))
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("identity unavailable"));
    }

    #[test]
    fn descendant_identity_validation_accepts_only_a_stable_creation_ordered_tree() {
        let root = WindowsProcessIdentity::new(9, 100);
        let child = WindowsProcessIdentity::new(10, 200);
        let grandchild = WindowsProcessIdentity::new(11, 300);
        let first = HashMap::from([(9, 1), (10, 9), (11, 10)]);
        let second = first.clone();
        let opened = HashMap::from([(10, child), (11, grandchild)]);

        assert_eq!(
            validate_windows_descendant_identities(root, &first, &second, &opened).unwrap(),
            vec![child, grandchild]
        );
    }

    #[test]
    fn descendant_identity_validation_rejects_pid_reuse_and_snapshot_changes() {
        let root = WindowsProcessIdentity::new(9, 200);
        let stale_child = WindowsProcessIdentity::new(10, 100);
        let stable = HashMap::from([(9, 1), (10, 9)]);
        let opened = HashMap::from([(10, stale_child)]);

        let stale_parent_error =
            validate_windows_descendant_identities(root, &stable, &stable, &opened).unwrap_err();
        assert!(stale_parent_error.to_string().contains("creation order"));

        let changed = HashMap::from([(9, 1), (10, 8)]);
        let changed_error =
            validate_windows_descendant_identities(root, &stable, &changed, &opened).unwrap_err();
        assert!(changed_error.to_string().contains("changed"));

        let missing_handle_error =
            validate_windows_descendant_identities(root, &stable, &stable, &HashMap::new())
                .unwrap_err();
        assert!(missing_handle_error.to_string().contains("stable handle"));
    }

    #[tokio::test]
    async fn process_tree_identity_failure_waits_without_termination() {
        let terminated = Arc::new(AtomicUsize::new(0));
        let root_waited = Arc::new(AtomicUsize::new(0));
        let descendants_waited = Arc::new(AtomicUsize::new(0));
        let terminate_count = Arc::clone(&terminated);
        let root_wait_count = Arc::clone(&root_waited);
        let descendant_wait_count = Arc::clone(&descendants_waited);

        let error = cleanup_confirmed_process_tree_with(
            "root",
            Err(anyhow::anyhow!("snapshot unavailable")),
            move |_| {
                let terminate_count = Arc::clone(&terminate_count);
                async move {
                    terminate_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            move |root| {
                let root_wait_count = Arc::clone(&root_wait_count);
                async move {
                    assert_eq!(root, "root");
                    root_wait_count.fetch_add(1, Ordering::SeqCst);
                }
            },
            move |_| {
                let descendant_wait_count = Arc::clone(&descendant_wait_count);
                async move {
                    descendant_wait_count.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await
        .unwrap_err();

        assert!(format!("{error:#}").contains("snapshot unavailable"));
        assert_eq!(terminated.load(Ordering::SeqCst), 0);
        assert_eq!(root_waited.load(Ordering::SeqCst), 1);
        assert_eq!(descendants_waited.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_tree_termination_failure_waits_for_root_and_descendants() {
        let root_waited = Arc::new(AtomicUsize::new(0));
        let descendants_waited = Arc::new(AtomicUsize::new(0));
        let root_wait_count = Arc::clone(&root_waited);
        let descendant_wait_count = Arc::clone(&descendants_waited);

        let error = cleanup_confirmed_process_tree_with(
            "root",
            Ok(vec!["child", "grandchild"]),
            |_| async { anyhow::bail!("termination denied") },
            move |_| {
                let root_wait_count = Arc::clone(&root_wait_count);
                async move {
                    root_wait_count.fetch_add(1, Ordering::SeqCst);
                }
            },
            move |_| {
                let descendant_wait_count = Arc::clone(&descendant_wait_count);
                async move {
                    descendant_wait_count.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("termination denied"));
        assert_eq!(root_waited.load(Ordering::SeqCst), 1);
        assert_eq!(descendants_waited.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn descendant_ownership_only_accepts_tracked_process_tree() {
        let parents = HashMap::from([(10, 9), (11, 10), (12, 8), (20, 21), (21, 20)]);

        assert!(process_descends_from(10, 9, &parents));
        assert!(process_descends_from(11, 9, &parents));
        assert!(!process_descends_from(12, 9, &parents));
        assert!(!process_descends_from(20, 9, &parents));
    }

    #[test]
    fn cleanup_result_reports_every_failed_step() {
        let mut errors = Vec::new();
        record_cleanup_result(&mut errors, "first", Err(anyhow::anyhow!("one")));
        record_cleanup_result(&mut errors, "success", Ok(()));
        record_cleanup_result(&mut errors, "second", Err(anyhow::anyhow!("two")));

        let message = finish_cleanup(errors).unwrap_err().to_string();

        assert!(message.contains("first: one"));
        assert!(message.contains("second: two"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn linux_process_termination_waits_for_exit() {
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30").process_group(0);
        let mut child = command.spawn().unwrap();
        let process_group = child.id().unwrap();
        let reaper = tokio::spawn(async move { child.wait().await.unwrap() });

        terminate_linux_process_group_and_wait(process_group)
            .await
            .unwrap();

        assert!(reaper.is_finished());
        assert!(!reaper.await.unwrap().success());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_process_group_parser_handles_spaces_and_parentheses_in_name() {
        assert_eq!(
            linux_process_group_from_stat("123 (Codex ) worker) S 10 456 789"),
            Some(456)
        );
    }

    #[test]
    fn http_body_framing_rejects_ambiguous_or_unsupported_headers() {
        let conflict = http_body_framing(
            b"POST / HTTP/1.1\r\nContent-Length: 4\r\nTransfer-Encoding: chunked",
        )
        .unwrap_err();
        assert_eq!(conflict.status(), "400 Bad Request");

        let unsupported =
            http_body_framing(b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip").unwrap_err();
        assert_eq!(unsupported.status(), "400 Bad Request");

        let multiple = http_body_framing(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\nTransfer-Encoding: chunked",
        )
        .unwrap_err();
        assert_eq!(multiple.status(), "400 Bad Request");
    }

    #[test]
    fn chunked_decoder_accepts_exact_body_limit_and_rejects_one_byte_more() {
        let mut exact = format!("{:X}\r\n", MAX_HTTP_BODY_BYTES).into_bytes();
        exact.resize(exact.len() + MAX_HTTP_BODY_BYTES, b'a');
        exact.extend_from_slice(b"\r\n0\r\n\r\n");
        let ChunkedBody::Complete(decoded) = decode_chunked_body(&exact).unwrap() else {
            panic!("expected complete chunked body");
        };
        assert_eq!(decoded.len(), MAX_HTTP_BODY_BYTES);

        let oversized = format!("{:X}\r\n", MAX_HTTP_BODY_BYTES + 1).into_bytes();
        let error = decode_chunked_body(&oversized).unwrap_err();
        assert_eq!(error.status(), "413 Payload Too Large");
    }

    #[test]
    fn chunked_decoder_handles_extensions_trailers_and_every_partial_prefix() {
        let encoded = b"3;name=value\r\n\x00\x80\xff\r\n2\r\nAB\r\n0\r\nX-Trace: yes\r\n\r\n";
        for prefix_len in 0..encoded.len() {
            assert!(matches!(
                scan_chunked_body(&encoded[..prefix_len]).unwrap(),
                ChunkedBodyScan::Incomplete
            ));
        }

        assert!(matches!(
            scan_chunked_body(encoded).unwrap(),
            ChunkedBodyScan::Complete
        ));
        let ChunkedBody::Complete(decoded) = decode_chunked_body(encoded).unwrap() else {
            panic!("expected complete chunked body");
        };
        assert_eq!(decoded, [0x00, 0x80, 0xff, b'A', b'B']);
    }

    #[test]
    fn chunked_decoder_rejects_oversized_size_lines_and_trailers() {
        let oversized_size_line = vec![b'f'; MAX_HTTP_HEADER_BYTES + 1];
        let error = scan_chunked_body(&oversized_size_line).unwrap_err();
        assert_eq!(error.status(), "400 Bad Request");

        let mut oversized_trailer = b"0\r\nX-Large: ".to_vec();
        oversized_trailer.resize(MAX_HTTP_HEADER_BYTES + 16, b'a');
        let error = scan_chunked_body(&oversized_trailer).unwrap_err();
        assert_eq!(error.status(), "400 Bad Request");
    }

    #[test]
    fn content_length_body_accepts_exact_limit_and_rejects_one_byte_more() {
        let exact = vec![b'a'; MAX_HTTP_BODY_BYTES];
        assert_eq!(
            content_length_body(&exact, MAX_HTTP_BODY_BYTES)
                .unwrap()
                .len(),
            MAX_HTTP_BODY_BYTES
        );

        let error = content_length_body(&[], MAX_HTTP_BODY_BYTES + 1).unwrap_err();
        assert_eq!(error.status(), "413 Payload Too Large");
    }

    #[tokio::test]
    async fn helper_returns_400_for_ambiguous_body_framing() {
        let response = send_raw_helper_request(
            b"POST /v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        )
        .await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400 Bad Request"));
    }

    #[tokio::test]
    async fn helper_returns_413_before_reading_oversized_content_length_body() {
        let request = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {}\r\n\r\n",
            MAX_HTTP_BODY_BYTES + 1
        );
        let response = send_raw_helper_request(request.as_bytes()).await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 413 Payload Too Large"));
    }

    #[tokio::test]
    async fn helper_returns_400_for_oversized_headers() {
        let mut request = b"GET /backend/status HTTP/1.1\r\nX-Large: ".to_vec();
        request.resize(MAX_HTTP_HEADER_BYTES + 1, b'a');
        request.extend_from_slice(b"\r\n\r\n");
        let response = send_raw_helper_request(&request).await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400 Bad Request"));
    }

    #[tokio::test]
    async fn helper_shutdown_closes_in_flight_connections() {
        let hooks = DefaultLaunchHooks::default();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        hooks.start_helper(port).await.unwrap();

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        client
            .write_all(
                b"POST /backend/status HTTP/1.1\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{",
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let active_connections = hooks
                    .helper
                    .lock()
                    .await
                    .as_ref()
                    .map(|runtime| runtime.active_connections.load(Ordering::Acquire))
                    .unwrap_or_default();
                if active_connections == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("helper must accept the in-flight connection before shutdown");

        hooks.shutdown_helper(port).await;

        let mut byte = [0_u8; 1];
        let read = tokio::time::timeout(std::time::Duration::from_secs(1), client.read(&mut byte))
            .await
            .expect("helper shutdown must close an in-flight connection");
        match read {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) => {}
            other => panic!("expected a closed helper connection, got {other:?}"),
        }
    }

    async fn send_raw_helper_request(request: &[u8]) -> Vec<u8> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        helper.await.unwrap();
        response
    }

    #[tokio::test]
    async fn helper_decodes_fragmented_chunked_binary_multipart_body() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfiles": [{
                "id": "audio",
                "name": "Audio",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-test",
                "protocol": "chatCompletions",
                "relayMode": "mixedApi"
            }],
            "activeRelayId": "audio"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let boundary = "codex-binary-boundary";
        let mut multipart = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-4o-mini-transcribe\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"binary.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .into_bytes();
        multipart.extend_from_slice(&[0x00, 0x80, 0xff, b'A', b'B']);
        multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let expected_body = multipart.clone();

        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "upstream request ended before body completed");
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none() {
                    if let Some(header_end) = find_header_end(&request) {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        expected_len = Some(header_end + 4 + content_length);
                    }
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let header_end = find_header_end(&request).unwrap();
            let body = request[header_end + 4..].to_vec();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"text\":\"ok\"}",
                )
                .await
                .unwrap();
            body
        });

        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let headers = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nHost: {helper_addr}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        for fragment in headers.as_bytes().chunks(7) {
            client.write_all(fragment).await.unwrap();
        }
        for fragment in multipart.chunks(11) {
            let chunk_header = format!("{:X}\r\n", fragment.len());
            client.write_all(chunk_header.as_bytes()).await.unwrap();
            client.write_all(fragment).await.unwrap();
            client.write_all(b"\r\n").await.unwrap();
        }
        client.write_all(b"0\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        helper.await.unwrap();
        assert_eq!(upstream.await.unwrap(), expected_body);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[test]
    fn post_launch_guard_stops_after_stable_ready_artifacts() {
        let artifacts = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: false,
        };

        assert!(!should_stop_post_launch_computer_use_guard(2, &artifacts));
        assert!(should_stop_post_launch_computer_use_guard(3, &artifacts));
    }

    #[test]
    fn post_launch_guard_keeps_retrying_until_artifacts_are_ready() {
        let missing_notify = crate::computer_use_guard::GuardArtifacts {
            notify_exe: None,
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: false,
        };
        let missing_marketplace = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: None,
            sky_package_json: None,
            runtime_exports_needed: false,
        };
        let missing_runtime_package = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: true,
        };

        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_notify
        ));
        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_marketplace
        ));
        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_runtime_package
        ));
    }
}

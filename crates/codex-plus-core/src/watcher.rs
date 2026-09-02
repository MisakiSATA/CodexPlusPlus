use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(any(windows, target_os = "linux"))]
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
pub use crate::windows_integration::WindowsProcessInfo;

pub const WATCHER_INTERVAL_SECONDS: f64 = 3.0;
pub const CDP_PROBE_TIMEOUT_SECONDS: f64 = 0.5;
pub const TAKEOVER_FAILURE_BACKOFF_SECONDS: f64 = 30.0;
pub const RESTART_STOP_WAIT_TIMEOUT_MS: u64 = 5_000;
const RESTART_STOP_WAIT_INTERVAL_MS: u64 = 100;
pub const WATCHER_RUN_NAME: &str = "CodexPlusPlusWatcher";
pub const WATCHER_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const WATCHER_STARTUP_SHORTCUT_NAME: &str = "CodexPlusPlusWatcher.lnk";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatcherInstallPlan {
    pub run_value_name: String,
    pub run_value: String,
    pub shortcut_name: String,
    pub shortcut_target: String,
    pub shortcut_arguments: String,
}

pub fn watcher_disabled_flag(root: &Path) -> PathBuf {
    root.join("watcher.disabled")
}

pub fn default_watcher_disabled_flag() -> PathBuf {
    watcher_disabled_flag(&crate::paths::default_app_state_dir())
}

pub fn enable_watcher_at(root: &Path) -> std::io::Result<()> {
    let flag = watcher_disabled_flag(root);
    if flag.exists() {
        std::fs::remove_file(flag)?;
    }
    Ok(())
}

pub fn disable_watcher_at(root: &Path) -> std::io::Result<()> {
    let flag = watcher_disabled_flag(root);
    if let Some(parent) = flag.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(flag, b"disabled")
}

pub fn enable_watcher() -> std::io::Result<()> {
    enable_watcher_at(&crate::paths::default_app_state_dir())
}

pub fn disable_watcher() -> std::io::Result<()> {
    disable_watcher_at(&crate::paths::default_app_state_dir())
}

pub fn cdp_listening(port: u16) -> bool {
    [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
    .into_iter()
    .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok())
}

pub fn build_spawn_launcher_command(launcher_path: &str, debug_port: u16) -> Vec<String> {
    vec![
        launcher_path.to_string(),
        "--debug-port".to_string(),
        debug_port.to_string(),
    ]
}

pub fn build_watcher_install_plan(launcher_path: PathBuf, debug_port: u16) -> WatcherInstallPlan {
    let launcher = launcher_path.to_string_lossy().to_string();
    let arguments = format!("--debug-port {debug_port}");
    WatcherInstallPlan {
        run_value_name: WATCHER_RUN_NAME.to_string(),
        run_value: format!("\"{launcher}\" {arguments}"),
        shortcut_name: WATCHER_STARTUP_SHORTCUT_NAME.to_string(),
        shortcut_target: launcher,
        shortcut_arguments: arguments,
    }
}

pub fn codex_process_ids<'a>(processes: impl IntoIterator<Item = (u32, &'a str)>) -> Vec<u32> {
    processes
        .into_iter()
        .filter_map(|(process_id, executable)| {
            is_windowsapps_codex_app_process(executable).then_some(process_id)
        })
        .collect()
}

pub fn packaged_activation_baseline_process_ids(processes: &[(u32, &str)]) -> Vec<u32> {
    let mut process_ids = processes
        .iter()
        .filter(|(_, executable)| {
            executable.eq_ignore_ascii_case("ChatGPT.exe")
                || executable.eq_ignore_ascii_case("Codex.exe")
        })
        .map(|(process_id, _)| *process_id)
        .collect::<Vec<_>>();
    process_ids.sort_unstable();
    process_ids.dedup();
    process_ids
}

fn is_windowsapps_codex_app_process(executable: &str) -> bool {
    let executable = executable.replace('/', "\\").to_ascii_lowercase();
    let Some((_, after_windows_apps)) = executable.split_once("\\windowsapps\\") else {
        return false;
    };
    let Some((package_name, after_package)) = after_windows_apps.split_once('\\') else {
        return false;
    };
    let supported_package = crate::app_paths::is_supported_windows_app_package_name(package_name)
        || package_name.starts_with("openai.chatgpt-desktop_");
    supported_package
        && after_package.starts_with("app\\")
        && !after_package.starts_with("app\\resources\\")
        && after_package
            .rsplit('\\')
            .next()
            .is_some_and(crate::app_paths::is_supported_app_executable_name)
}

pub fn filter_killable_launcher_processes<'a>(
    processes: impl IntoIterator<Item = (u32, u32, &'a str)>,
    current_process_id: u32,
) -> Vec<u32> {
    let processes = processes.into_iter().collect::<Vec<_>>();
    let parents = processes
        .iter()
        .map(|(process_id, parent_process_id, _)| (*process_id, *parent_process_id))
        .collect::<HashMap<_, _>>();
    let mut protected = HashSet::new();
    let mut cursor = current_process_id;
    while cursor != 0 && protected.insert(cursor) {
        cursor = parents.get(&cursor).copied().unwrap_or(0);
    }
    processes
        .into_iter()
        .filter(|(process_id, _, exe_file)| {
            !protected.contains(process_id)
                && (exe_file.eq_ignore_ascii_case("codex-plus-plus.exe")
                    || *exe_file == "codex-plus-plus")
        })
        .map(|(process_id, _, _)| process_id)
        .collect()
}

pub fn should_recover_stale_launcher(has_codex_process: bool, cdp_listening: bool) -> bool {
    !has_codex_process && !cdp_listening
}

pub fn process_ids_still_running(
    expected: &[u32],
    running: impl IntoIterator<Item = u32>,
) -> Vec<u32> {
    let expected = expected.iter().copied().collect::<HashSet<_>>();
    running
        .into_iter()
        .filter(|process_id| expected.contains(process_id))
        .collect()
}

pub fn launcher_stop_complete(remaining_process_ids: &[u32], guard_port_listening: bool) -> bool {
    remaining_process_ids.is_empty() && !guard_port_listening
}

#[cfg(windows)]
pub fn install_watcher(launcher_path: &Path, debug_port: u16) -> anyhow::Result<()> {
    let plan = build_watcher_install_plan(launcher_path.to_path_buf(), debug_port);
    crate::windows_integration::set_current_user_string_value(
        WATCHER_RUN_KEY,
        &plan.run_value_name,
        &plan.run_value,
    )?;
    create_startup_shortcut(launcher_path, &plan.shortcut_arguments)?;
    spawn_launcher(launcher_path, debug_port);
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install_watcher(launcher_path: &Path, debug_port: u16) -> anyhow::Result<()> {
    let Some(config_home) = crate::install::linux::default_autostart_config_home() else {
        anyhow::bail!("无法解析当前用户的配置目录");
    };
    let entry_path = crate::install::linux::watcher_autostart_path(&config_home);
    if let Some(parent) = entry_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &entry_path,
        crate::install::linux::build_watcher_autostart_entry(launcher_path, debug_port),
    )?;
    spawn_launcher(launcher_path, debug_port);
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn install_watcher(_launcher_path: &Path, _debug_port: u16) -> anyhow::Result<()> {
    anyhow::bail!("watcher install is only supported on Windows and Linux")
}

#[cfg(windows)]
pub fn uninstall_watcher() -> anyhow::Result<()> {
    let _ =
        crate::windows_integration::delete_current_user_value(WATCHER_RUN_KEY, WATCHER_RUN_NAME);
    if let Some(shortcut) = startup_shortcut_path() {
        let _ = std::fs::remove_file(shortcut);
    }
    stop_launcher_processes();
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall_watcher() -> anyhow::Result<()> {
    if let Some(config_home) = crate::install::linux::default_autostart_config_home() {
        let entry_path = crate::install::linux::watcher_autostart_path(&config_home);
        if entry_path.exists() {
            let _ = std::fs::remove_file(&entry_path);
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn uninstall_watcher() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn find_codex_processes() -> Vec<u32> {
    let processes: Vec<_> = crate::windows_integration::enumerate_processes()
        .into_iter()
        .filter(|process| crate::app_paths::is_supported_app_executable_name(&process.exe_file))
        .collect();
    find_codex_processes_from_snapshot(&processes)
}

/// Filter the list of already enumerated Windows processes for Codex processes.
/// Exposed so the Windows-specific logic can be unit-tested without scanning the live system.
#[cfg(windows)]
pub fn find_codex_processes_from_snapshot(
    processes: &[crate::windows_integration::WindowsProcessInfo],
) -> Vec<u32> {
    let mut ids = codex_process_ids(
        processes
            .iter()
            .filter_map(|process| {
                process
                    .executable_path
                    .as_deref()
                    .map(|path| (process.process_id, path.to_string_lossy().to_string()))
            })
            .collect::<Vec<_>>()
            .iter()
            .map(|(pid, path)| (*pid, path.as_str())),
    );

    // Local/portable installs use Codex.exe as the Electron main process. Do not match
    // lowercase codex.exe here; that is commonly the CLI binary. ChatGPT.exe is accepted
    // only for packaged Store apps above, because the standalone ChatGPT app can be a
    // normal ChatGPT session rather than Codex.
    for process in processes {
        if process.exe_file == "Codex.exe" {
            ids.push(process.process_id);
        }
    }

    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Return desktop processes that can write Codex task state while a destructive
/// session-index cleanup is running. This is intentionally stricter than the
/// watcher filter: any supported ChatGPT desktop process blocks deletion,
/// including portable installs outside WindowsApps.
#[cfg(windows)]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    find_session_index_cleanup_blocking_processes_from_snapshot(
        &crate::windows_integration::enumerate_processes(),
    )
}

#[cfg(windows)]
pub fn find_session_index_cleanup_blocking_processes_from_snapshot(
    processes: &[crate::windows_integration::WindowsProcessInfo],
) -> Vec<u32> {
    let mut ids = processes
        .iter()
        .filter(|process| process.exe_file == "Codex.exe" || process.exe_file == "ChatGPT.exe")
        .map(|process| process.process_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "macos")]
pub fn find_codex_processes() -> Vec<u32> {
    let mut ids = ["Codex", "ChatGPT"]
        .into_iter()
        .flat_map(|name| {
            std::process::Command::new("pgrep")
                .args(["-x", name])
                .output()
                .ok()
                .into_iter()
                .flat_map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
        })
        .filter_map(|value| value.trim().parse::<u32>().ok())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "macos")]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    find_codex_processes()
}

#[cfg(target_os = "linux")]
pub fn find_codex_processes() -> Vec<u32> {
    find_linux_codex_processes_from_proc(Path::new("/proc"))
}

#[cfg(target_os = "linux")]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    find_codex_processes()
}

#[cfg(target_os = "linux")]
pub fn find_linux_codex_processes_from_proc(proc_root: &Path) -> Vec<u32> {
    let mut ids = std::fs::read_dir(proc_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let process_id = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let command_line = std::fs::read(entry.path().join("cmdline")).ok()?;
            linux_codex_main_process_command(&command_line).then_some(process_id)
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "linux")]
pub fn find_linux_launcher_processes_from_proc(
    proc_root: &Path,
    current_process_id: u32,
) -> Vec<u32> {
    let processes = std::fs::read_dir(proc_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let process_id = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let parent_process_id = linux_parent_process_id(&entry.path().join("stat"))?;
            let executable_path = std::fs::read_link(entry.path().join("exe")).ok()?;
            let executable_name = executable_path.file_name()?.to_str()?.to_string();
            Some((process_id, parent_process_id, executable_name))
        })
        .collect::<Vec<_>>();
    filter_killable_launcher_processes(
        processes
            .iter()
            .map(|(process_id, parent_process_id, executable_name)| {
                (*process_id, *parent_process_id, executable_name.as_str())
            }),
        current_process_id,
    )
}

#[cfg(target_os = "linux")]
fn linux_parent_process_id(stat_path: &Path) -> Option<u32> {
    let stat = std::fs::read_to_string(stat_path).ok()?;
    let after_name = stat.rsplit_once(") ")?.1;
    after_name.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn linux_codex_main_process_command(command_line: &[u8]) -> bool {
    let arguments = command_line
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .filter_map(|argument| std::str::from_utf8(argument).ok())
        .collect::<Vec<_>>();
    let tokens = if arguments.len() == 1 {
        arguments[0].split_ascii_whitespace().collect::<Vec<_>>()
    } else {
        arguments.clone()
    };
    if tokens
        .iter()
        .any(|argument| argument.starts_with("--type="))
    {
        return false;
    }
    tokens.first() == Some(&"/usr/lib/chatgpt/ChatGPT")
        || (tokens.first().is_some_and(|executable| {
            Path::new(executable)
                .file_name()
                .and_then(|name| name.to_str())
                == Some("electron")
        }) && tokens.iter().any(|argument| {
            !argument.starts_with("--")
                && argument.ends_with("/resources/app.asar")
                && argument.contains("/openai-codex-desktop/")
        }))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn find_codex_processes() -> Vec<u32> {
    Vec::new()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    Vec::new()
}

#[cfg(windows)]
pub fn stop_launcher_processes() {
    let processes = crate::windows_integration::enumerate_processes();
    let killable = filter_killable_launcher_processes(
        processes.iter().map(|process| {
            (
                process.process_id,
                process.parent_process_id,
                process.exe_file.as_str(),
            )
        }),
        std::process::id(),
    );
    for process_id in killable {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(target_os = "linux")]
pub fn stop_launcher_processes() {
    terminate_linux_processes(&find_linux_launcher_processes_from_proc(
        Path::new("/proc"),
        std::process::id(),
    ));
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn stop_launcher_processes() {}

#[cfg(windows)]
pub fn stop_launcher_processes_and_wait() {
    let processes = crate::windows_integration::enumerate_processes();
    let killable = filter_killable_launcher_processes(
        processes.iter().map(|process| {
            (
                process.process_id,
                process.parent_process_id,
                process.exe_file.as_str(),
            )
        }),
        std::process::id(),
    );
    terminate_and_wait_for_exit(
        killable,
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(target_os = "linux")]
pub fn stop_launcher_processes_and_wait() {
    terminate_linux_launcher_processes_and_wait(
        find_linux_launcher_processes_from_proc(Path::new("/proc"), std::process::id()),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn stop_launcher_processes_and_wait() {}

#[cfg(windows)]
pub fn stop_codex_processes() {
    for process_id in find_codex_processes() {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(target_os = "linux")]
pub fn stop_codex_processes() {
    terminate_linux_processes(&find_codex_processes());
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn stop_codex_processes() {}

#[cfg(windows)]
pub fn stop_codex_processes_and_wait() {
    terminate_and_wait_for_exit(
        find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(target_os = "linux")]
pub fn stop_codex_processes_and_wait() {
    terminate_and_wait_for_exit(
        find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn stop_codex_processes_and_wait() {}

#[cfg(windows)]
fn terminate_and_wait_for_exit(process_ids: Vec<u32>, timeout_ms: u64, interval_ms: u64) {
    if process_ids.is_empty() {
        return;
    }
    for process_id in &process_ids {
        let _ = crate::windows_integration::terminate_process(*process_id);
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let running_process_ids = crate::windows_integration::enumerate_processes()
            .into_iter()
            .map(|process| process.process_id);
        let remaining = process_ids_still_running(&process_ids, running_process_ids);
        if remaining.is_empty() || std::time::Instant::now() >= deadline {
            if !remaining.is_empty() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "watcher.stop_wait_timeout",
                    serde_json::json!({
                        "remaining_process_ids": remaining,
                        "timeout_ms": timeout_ms
                    }),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

#[cfg(target_os = "linux")]
fn terminate_linux_launcher_processes_and_wait(
    process_ids: Vec<u32>,
    timeout_ms: u64,
    interval_ms: u64,
) {
    if process_ids.is_empty() && !cdp_listening(crate::ports::launcher_guard_port()) {
        return;
    }
    terminate_linux_processes(&process_ids);
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining =
            find_linux_launcher_processes_from_proc(Path::new("/proc"), std::process::id())
                .into_iter()
                .filter(|process_id| process_ids.contains(process_id))
                .collect::<Vec<_>>();
        let guard_port_listening = cdp_listening(crate::ports::launcher_guard_port());
        if launcher_stop_complete(&remaining, guard_port_listening)
            || std::time::Instant::now() >= deadline
        {
            if !launcher_stop_complete(&remaining, guard_port_listening) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "watcher.launcher_stop_wait_timeout",
                    serde_json::json!({
                        "remaining_process_ids": remaining,
                        "guard_port": crate::ports::launcher_guard_port(),
                        "guard_port_listening": guard_port_listening,
                        "timeout_ms": timeout_ms
                    }),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

#[cfg(target_os = "linux")]
fn terminate_and_wait_for_exit(process_ids: Vec<u32>, timeout_ms: u64, interval_ms: u64) {
    if process_ids.is_empty() {
        return;
    }
    terminate_linux_processes(&process_ids);
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining = process_ids
            .iter()
            .copied()
            .filter(|process_id| Path::new("/proc").join(process_id.to_string()).exists())
            .collect::<Vec<_>>();
        if remaining.is_empty() || std::time::Instant::now() >= deadline {
            if !remaining.is_empty() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "watcher.stop_wait_timeout",
                    serde_json::json!({
                        "remaining_process_ids": remaining,
                        "timeout_ms": timeout_ms
                    }),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

#[cfg(target_os = "linux")]
fn terminate_linux_processes(process_ids: &[u32]) {
    if process_ids.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg("-TERM")
        .args(process_ids.iter().map(u32::to_string))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn create_startup_shortcut(launcher_path: &Path, arguments: &str) -> anyhow::Result<()> {
    let Some(shortcut_path) = startup_shortcut_path() else {
        anyhow::bail!("无法定位 Windows 启动目录")
    };
    crate::windows_integration::create_shortcut(&crate::windows_integration::ShortcutSpec {
        path: shortcut_path,
        target: launcher_path.to_path_buf(),
        arguments: arguments.to_string(),
        working_directory: launcher_path.parent().map(Path::to_path_buf),
        description: "Codex++ watcher".to_string(),
        icon: None,
        show_minimized: true,
    })
}

#[cfg(any(windows, target_os = "linux"))]
fn spawn_launcher(launcher_path: &Path, debug_port: u16) {
    let command = build_spawn_launcher_command(&launcher_path.to_string_lossy(), debug_port);
    if let Some((exe, args)) = command.split_first() {
        let mut command = Command::new(exe);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        }
        let _ = command.spawn();
    }
}

#[cfg(windows)]
fn startup_shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(WATCHER_STARTUP_SHORTCUT_NAME)
    })
}

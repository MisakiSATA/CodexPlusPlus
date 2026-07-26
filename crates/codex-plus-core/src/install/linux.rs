use std::path::{Path, PathBuf};

use super::{
    InstallOptions, MANAGER_BINARY, MANAGER_NAME, SILENT_BINARY, SILENT_NAME,
    install_root_or_default, option_or_current_exe,
};

pub const SILENT_DESKTOP_FILE: &str = "codex-plus-plus.desktop";
pub const MANAGER_DESKTOP_FILE: &str = "codex-plus-plus-manager.desktop";
pub const WATCHER_AUTOSTART_FILE: &str = "codex-plus-plus-watcher.desktop";
pub const ICON_NAME: &str = "codex-plus-plus";
pub const ICON_FILE: &str = "codex-plus-plus.png";
pub const INSTALL_MANIFEST_FILE: &str = "install-manifest.json";
pub const INSTALL_MANAGED_BY: &str = "Codex++ user install";
pub const APP_LIBRARY_DIR_NAME: &str = "codex-plus-plus";
pub const CURRENT_LINK_NAME: &str = "current";

const ICON_BYTES: &[u8] = include_bytes!("../../../../assets/images/codex-plus-plus.png");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxDesktopEntry {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxEntrypointPlan {
    pub silent: LinuxDesktopEntry,
    pub manager: LinuxDesktopEntry,
    pub launcher_path: PathBuf,
    pub manager_path: PathBuf,
}

/// 用户级安装涉及的根目录，全部位于当前用户可写的 XDG 位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxInstallRoots {
    /// `.desktop` 入口目录，通常是 `~/.local/share/applications`。
    pub applications_dir: PathBuf,
    /// 图标主题根目录，通常是 `~/.local/share/icons`。
    pub icons_dir: PathBuf,
    /// 程序版本库根目录，通常是 `~/.local/lib/codex-plus-plus`。
    pub library_root: PathBuf,
}

/// 完成用户级安装后各稳定路径的汇总。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxUserInstall {
    pub launcher_path: PathBuf,
    pub manager_path: PathBuf,
    pub version_dir: PathBuf,
    pub current_link: PathBuf,
    pub silent_entry: PathBuf,
    pub manager_entry: PathBuf,
    pub icon_path: PathBuf,
}

pub fn build_entrypoint_plan(options: &InstallOptions) -> LinuxEntrypointPlan {
    let root = install_root_or_default(options);
    let launcher_path = option_or_current_exe(&options.launcher_path, SILENT_BINARY);
    let manager_path = option_or_current_exe(&options.manager_path, MANAGER_BINARY);
    build_entrypoint_plan_for_binaries(&root, &launcher_path, &manager_path)
}

fn build_entrypoint_plan_for_binaries(
    applications_dir: &Path,
    launcher_path: &Path,
    manager_path: &Path,
) -> LinuxEntrypointPlan {
    LinuxEntrypointPlan {
        silent: LinuxDesktopEntry {
            path: applications_dir.join(SILENT_DESKTOP_FILE),
            contents: desktop_entry(
                SILENT_NAME,
                "Launch OpenAI Codex with Codex++ enhancements",
                launcher_path,
                true,
            ),
        },
        manager: LinuxDesktopEntry {
            path: applications_dir.join(MANAGER_DESKTOP_FILE),
            contents: desktop_entry(
                MANAGER_NAME,
                "Manage Codex++ providers, models and enhancements",
                manager_path,
                false,
            ),
        },
        launcher_path: launcher_path.to_path_buf(),
        manager_path: manager_path.to_path_buf(),
    }
}

/// 图标安装位置：`<icons_dir>/hicolor/256x256/apps/codex-plus-plus.png`。
pub fn icon_install_path(icons_dir: &Path) -> PathBuf {
    icons_dir
        .join("hicolor")
        .join("256x256")
        .join("apps")
        .join(ICON_FILE)
}

/// watcher 自启动入口位置：`<config_home>/autostart/codex-plus-plus-watcher.desktop`。
pub fn watcher_autostart_path(config_home: &Path) -> PathBuf {
    config_home.join("autostart").join(WATCHER_AUTOSTART_FILE)
}

pub fn build_watcher_autostart_entry(launcher_path: &Path, debug_port: u16) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Codex++ Watcher\nComment=Keep Codex++ enhancements attached to the Codex desktop app\nExec={} --debug-port {debug_port}\nIcon={ICON_NAME}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_path(launcher_path),
    )
}

#[cfg(target_os = "linux")]
pub fn default_install_roots() -> Option<LinuxInstallRoots> {
    let base = directories::BaseDirs::new()?;
    Some(LinuxInstallRoots {
        applications_dir: base.data_local_dir().join("applications"),
        icons_dir: base.data_local_dir().join("icons"),
        library_root: base.home_dir().join(".local").join("lib").join(APP_LIBRARY_DIR_NAME),
    })
}

#[cfg(target_os = "linux")]
pub fn default_autostart_config_home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.config_dir().to_path_buf())
}

/// 仅写入指向给定二进制的 `.desktop` 入口，不复制程序本体。
/// 保留给测试与「入口修复」这类不改动已装程序的场景。
#[cfg(target_os = "linux")]
pub fn install_desktop_entries(options: &InstallOptions) -> anyhow::Result<()> {
    let plan = build_entrypoint_plan(options);
    write_entrypoint_plan(&plan)
}

#[cfg(target_os = "linux")]
fn write_entrypoint_plan(plan: &LinuxEntrypointPlan) -> anyhow::Result<()> {
    if !plan.launcher_path.is_file() {
        anyhow::bail!(
            "找不到 Codex++ 启动器：{}",
            plan.launcher_path.to_string_lossy()
        );
    }
    if !plan.manager_path.is_file() {
        anyhow::bail!(
            "找不到 Codex++ 管理工具：{}",
            plan.manager_path.to_string_lossy()
        );
    }
    if let Some(parent) = plan.silent.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plan.silent.path, &plan.silent.contents)?;
    std::fs::write(&plan.manager.path, &plan.manager.contents)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall_desktop_entries(options: &InstallOptions) -> anyhow::Result<()> {
    let plan = build_entrypoint_plan(options);
    for path in [&plan.silent.path, &plan.manager.path] {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// 完整的用户级安装闭环：把二进制复制进版本目录、原子切换 `current`
/// 链接、安装图标并写入始终指向稳定路径的 `.desktop` 入口。
#[cfg(target_os = "linux")]
pub fn install_user_scoped(
    options: &InstallOptions,
    roots: &LinuxInstallRoots,
    version: &str,
) -> anyhow::Result<LinuxUserInstall> {
    let launcher_source = option_or_current_exe(&options.launcher_path, SILENT_BINARY);
    let manager_source = option_or_current_exe(&options.manager_path, MANAGER_BINARY);
    if !launcher_source.is_file() {
        anyhow::bail!(
            "找不到 Codex++ 启动器：{}",
            launcher_source.to_string_lossy()
        );
    }
    if !manager_source.is_file() {
        anyhow::bail!(
            "找不到 Codex++ 管理工具：{}",
            manager_source.to_string_lossy()
        );
    }

    let version_dir = install_versioned_files(
        &roots.library_root,
        version,
        &launcher_source,
        &manager_source,
    )?;
    let current_link = activate_version(&roots.library_root, &version_dir)?;

    let launcher_path = current_link.join("bin").join(SILENT_BINARY);
    let manager_path = current_link.join("bin").join(MANAGER_BINARY);
    let applications_dir = options
        .install_root
        .clone()
        .unwrap_or_else(|| roots.applications_dir.clone());
    let plan = build_entrypoint_plan_for_binaries(&applications_dir, &launcher_path, &manager_path);
    write_entrypoint_plan(&plan)?;

    let icon_path = icon_install_path(&roots.icons_dir);
    write_file_atomically(&icon_path, ICON_BYTES)?;

    Ok(LinuxUserInstall {
        launcher_path,
        manager_path,
        version_dir,
        current_link,
        silent_entry: plan.silent.path,
        manager_entry: plan.manager.path,
        icon_path,
    })
}

/// 卸载入口文件（`.desktop`、图标、自启动）。程序版本目录由
/// `uninstall_user_application` 按 manifest 单独移除。
#[cfg(target_os = "linux")]
pub fn uninstall_user_scoped(
    options: &InstallOptions,
    roots: &LinuxInstallRoots,
    config_home: Option<&Path>,
) -> anyhow::Result<()> {
    let applications_dir = options
        .install_root
        .clone()
        .unwrap_or_else(|| roots.applications_dir.clone());
    for file_name in [SILENT_DESKTOP_FILE, MANAGER_DESKTOP_FILE] {
        let entry = applications_dir.join(file_name);
        if entry.exists() {
            std::fs::remove_file(&entry)?;
        }
    }
    let icon_path = icon_install_path(&roots.icons_dir);
    if icon_path.exists() {
        std::fs::remove_file(&icon_path)?;
    }
    if let Some(config_home) = config_home {
        let autostart = watcher_autostart_path(config_home);
        if autostart.exists() {
            std::fs::remove_file(&autostart)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_versioned_files(
    library_root: &Path,
    version: &str,
    launcher_source: &Path,
    manager_source: &Path,
) -> anyhow::Result<PathBuf> {
    let version_dir = library_root.join("versions").join(version);
    let bin_dir = version_dir.join("bin");
    let share_dir = version_dir.join("share");
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::create_dir_all(&share_dir)?;

    copy_executable_atomically(launcher_source, &bin_dir.join(SILENT_BINARY))?;
    copy_executable_atomically(manager_source, &bin_dir.join(MANAGER_BINARY))?;
    write_file_atomically(&share_dir.join("icon.png"), ICON_BYTES)?;

    let manifest = serde_json::json!({
        "managedBy": INSTALL_MANAGED_BY,
        "version": version,
        "files": [
            format!("bin/{SILENT_BINARY}"),
            format!("bin/{MANAGER_BINARY}"),
            "share/icon.png",
        ],
    });
    write_file_atomically(
        &version_dir.join(INSTALL_MANIFEST_FILE),
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    Ok(version_dir)
}

/// 用临时链接加原子重命名把 `current` 指向新的版本目录。
#[cfg(target_os = "linux")]
fn activate_version(library_root: &Path, version_dir: &Path) -> anyhow::Result<PathBuf> {
    let current_link = library_root.join(CURRENT_LINK_NAME);
    let relative_target = PathBuf::from("versions").join(
        version_dir
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("版本目录缺少名称"))?,
    );
    let staging_link = library_root.join(format!(".{CURRENT_LINK_NAME}-{}", std::process::id()));
    if staging_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&staging_link)?;
    }
    std::os::unix::fs::symlink(&relative_target, &staging_link)?;
    std::fs::rename(&staging_link, &current_link)?;
    Ok(current_link)
}

/// 按 manifest 移除受管版本目录与 `current` 链接。
/// 只删除 manifest 声明的文件，任何未受管文件都会保留并使目录留存。
#[cfg(target_os = "linux")]
pub fn uninstall_user_application(library_root: &Path) -> anyhow::Result<()> {
    let current_link = library_root.join(CURRENT_LINK_NAME);
    if current_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&current_link)?;
    }
    let versions_dir = library_root.join("versions");
    if versions_dir.is_dir() {
        for entry in std::fs::read_dir(&versions_dir)? {
            let version_dir = entry?.path();
            remove_manifest_owned_version(&version_dir)?;
        }
        let _ = std::fs::remove_dir(&versions_dir);
    }
    let _ = std::fs::remove_dir(library_root);
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_manifest_owned_version(version_dir: &Path) -> anyhow::Result<()> {
    let manifest_path = version_dir.join(INSTALL_MANIFEST_FILE);
    let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
        return Ok(());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return Ok(());
    };
    if manifest.get("managedBy").and_then(|value| value.as_str()) != Some(INSTALL_MANAGED_BY) {
        return Ok(());
    }
    for file in manifest
        .get("files")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
    {
        // manifest 内只允许相对路径，拒绝越界删除。
        let relative = Path::new(file);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            continue;
        }
        let target = version_dir.join(relative);
        if target.exists() {
            let _ = std::fs::remove_file(&target);
        }
    }
    let _ = std::fs::remove_file(&manifest_path);
    for sub_dir in ["bin", "share"] {
        let _ = std::fs::remove_dir(version_dir.join(sub_dir));
    }
    let _ = std::fs::remove_dir(version_dir);
    Ok(())
}

#[cfg(target_os = "linux")]
fn copy_executable_atomically(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let staging = staging_path(target)?;
    std::fs::copy(source, &staging)?;
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&staging, target)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_file_atomically(target: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staging = staging_path(target)?;
    std::fs::write(&staging, contents)?;
    std::fs::rename(&staging, target)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn staging_path(target: &Path) -> anyhow::Result<PathBuf> {
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("目标路径缺少文件名：{}", target.to_string_lossy()))?;
    Ok(target.with_file_name(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    )))
}

pub const UPDATE_ROLLBACK_FILE: &str = "update-rollback.json";

/// 判断二进制是否由系统包管理器安装（`.deb`/`.rpm`/pacman 布局）。
/// 这类安装的文件归包管理器所有，应用内自更新必须拒绝执行。
pub fn is_system_managed_install(executable: &Path) -> bool {
    ["/usr/", "/opt/"]
        .iter()
        .any(|prefix| executable.starts_with(prefix))
}

/// 校验摘要后把便携更新包分级解压为新版本目录并原子切换 `current`。
/// 旧的 `current` 目标写入回滚元数据，供 `rollback_update` 恢复。
#[cfg(target_os = "linux")]
pub fn install_update_archive(
    archive: &[u8],
    expected_sha256: &str,
    roots: &LinuxInstallRoots,
    version: &str,
) -> anyhow::Result<LinuxUserInstall> {
    use sha2::Digest;

    let actual = format!("{:x}", sha2::Sha256::digest(archive));
    if !actual.eq_ignore_ascii_case(expected_sha256.trim()) {
        anyhow::bail!("更新包 SHA-256 校验失败：期望 {expected_sha256}，实际 {actual}");
    }

    let versions_dir = roots.library_root.join("versions");
    std::fs::create_dir_all(&versions_dir)?;
    let staging_dir = versions_dir.join(format!(".staging-{version}-{}", std::process::id()));
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)?;
    }
    let extraction = extract_update_archive(archive, &staging_dir);
    if let Err(error) = extraction {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    // 校验并授予可执行权限，失败时清理暂存目录。
    if let Err(error) = finalize_staged_binaries(&staging_dir) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    let version_dir = versions_dir.join(version);
    let replaced_dir = versions_dir.join(format!(".replaced-{version}-{}", std::process::id()));
    if version_dir.exists() {
        std::fs::rename(&version_dir, &replaced_dir)?;
    }
    if let Err(error) = std::fs::rename(&staging_dir, &version_dir) {
        if replaced_dir.exists() {
            let _ = std::fs::rename(&replaced_dir, &version_dir);
        }
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(error.into());
    }
    if replaced_dir.exists() {
        let _ = std::fs::remove_dir_all(&replaced_dir);
    }

    write_rollback_metadata(&roots.library_root)?;
    let current_link = activate_version(&roots.library_root, &version_dir)?;

    let launcher_path = current_link.join("bin").join(SILENT_BINARY);
    let manager_path = current_link.join("bin").join(MANAGER_BINARY);
    Ok(LinuxUserInstall {
        launcher_path,
        manager_path,
        version_dir,
        current_link,
        silent_entry: roots.applications_dir.join(SILENT_DESKTOP_FILE),
        manager_entry: roots.applications_dir.join(MANAGER_DESKTOP_FILE),
        icon_path: icon_install_path(&roots.icons_dir),
    })
}

/// 把 `current` 恢复到上一次激活的版本目录。
#[cfg(target_os = "linux")]
pub fn rollback_update(library_root: &Path) -> anyhow::Result<PathBuf> {
    let metadata_path = library_root.join(UPDATE_ROLLBACK_FILE);
    let contents = std::fs::read_to_string(&metadata_path)
        .map_err(|error| anyhow::anyhow!("没有可用的回滚信息：{error}"))?;
    let metadata: serde_json::Value = serde_json::from_str(&contents)?;
    if metadata.get("managedBy").and_then(|value| value.as_str()) != Some(INSTALL_MANAGED_BY) {
        anyhow::bail!("回滚信息不受 Codex++ 管理，已拒绝");
    }
    let previous = metadata
        .get("previousTarget")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("回滚信息缺少上一版本目录"))?;
    let previous_relative = Path::new(previous);
    if previous_relative.is_absolute()
        || previous_relative
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        anyhow::bail!("回滚目标路径非法：{previous}");
    }
    let previous_dir = library_root.join(previous_relative);
    if !previous_dir.is_dir() {
        anyhow::bail!("上一版本目录不存在：{}", previous_dir.to_string_lossy());
    }
    let current_link = activate_version(library_root, &previous_dir)?;
    let _ = std::fs::remove_file(&metadata_path);
    Ok(current_link)
}

#[cfg(target_os = "linux")]
fn write_rollback_metadata(library_root: &Path) -> anyhow::Result<()> {
    let current_link = library_root.join(CURRENT_LINK_NAME);
    let Ok(previous_target) = std::fs::read_link(&current_link) else {
        // 首次安装没有旧版本，无需回滚信息。
        return Ok(());
    };
    let metadata = serde_json::json!({
        "managedBy": INSTALL_MANAGED_BY,
        "previousTarget": previous_target.to_string_lossy(),
    });
    write_file_atomically(
        &library_root.join(UPDATE_ROLLBACK_FILE),
        serde_json::to_string_pretty(&metadata)?.as_bytes(),
    )
}

/// 解压便携更新包。逐项拒绝绝对路径、越界、符号链接、重复目标
/// 以及 manifest 未声明的文件。
#[cfg(target_os = "linux")]
fn extract_update_archive(archive: &[u8], target_dir: &Path) -> anyhow::Result<()> {
    use std::collections::HashSet;
    use std::io::Read;

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))?;

    // 第一遍：读取并校验 manifest。
    let manifest_contents = {
        let mut file = zip
            .by_name(INSTALL_MANIFEST_FILE)
            .map_err(|_| anyhow::anyhow!("更新包缺少 {INSTALL_MANIFEST_FILE}"))?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        contents
    };
    let manifest: serde_json::Value = serde_json::from_str(&manifest_contents)?;
    if manifest.get("managedBy").and_then(|value| value.as_str()) != Some(INSTALL_MANAGED_BY) {
        anyhow::bail!("更新包 manifest 缺少 Codex++ 管理标记");
    }
    let allowed = manifest
        .get("files")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for required in [
        format!("bin/{SILENT_BINARY}"),
        format!("bin/{MANAGER_BINARY}"),
    ] {
        if !allowed.contains(&required) {
            anyhow::bail!("更新包 manifest 缺少必需文件：{required}");
        }
    }

    // 第二遍：逐项校验并写入暂存目录。
    std::fs::create_dir_all(target_dir)?;
    let mut seen = HashSet::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().to_string();
        let Some(relative) = entry.enclosed_name() else {
            anyhow::bail!("更新包内路径非法：{raw_name}");
        };
        if relative.is_absolute() {
            anyhow::bail!("更新包内路径非法：{raw_name}");
        }
        // 拒绝符号链接条目，防止解压后逃逸出版本目录。
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            anyhow::bail!("更新包内含符号链接：{raw_name}");
        }
        let normalized = relative.to_string_lossy().to_string();
        if !seen.insert(normalized.clone()) {
            anyhow::bail!("更新包内目标重复：{raw_name}");
        }
        if normalized != INSTALL_MANIFEST_FILE && !allowed.contains(&normalized) {
            anyhow::bail!("更新包内文件未在 manifest 声明：{raw_name}");
        }
        let target = target_dir.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut output = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn finalize_staged_binaries(staging_dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for binary in [SILENT_BINARY, MANAGER_BINARY] {
        let path = staging_dir.join("bin").join(binary);
        let metadata = std::fs::metadata(&path)
            .map_err(|_| anyhow::anyhow!("更新包缺少二进制：bin/{binary}"))?;
        if metadata.len() == 0 {
            anyhow::bail!("更新包内二进制为空：bin/{binary}");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// 尽力刷新桌面数据库、图标缓存并注册协议处理器。
/// 缺少这些工具只影响菜单刷新速度，因此仅返回警告而不视为安装失败。
#[cfg(target_os = "linux")]
pub fn refresh_desktop_integration(roots: &LinuxInstallRoots) -> Vec<String> {
    let mut warnings = Vec::new();
    let commands: [(&str, Vec<std::ffi::OsString>); 3] = [
        (
            "update-desktop-database",
            vec![roots.applications_dir.clone().into()],
        ),
        (
            "gtk-update-icon-cache",
            vec![
                "-f".into(),
                "-t".into(),
                roots.icons_dir.join("hicolor").into(),
            ],
        ),
        (
            "xdg-mime",
            vec![
                "default".into(),
                SILENT_DESKTOP_FILE.into(),
                "x-scheme-handler/codexplusplus".into(),
            ],
        ),
    ];
    for (program, args) in commands {
        let result = std::process::Command::new(program)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => warnings.push(format!("{program} 退出码 {status}")),
            Err(error) => warnings.push(format!("{program} 不可用：{error}")),
        }
    }
    warnings
}

fn desktop_entry(
    name: &str,
    comment: &str,
    executable: &std::path::Path,
    handles_urls: bool,
) -> String {
    let url_argument = if handles_urls { " %U" } else { "" };
    let mime_type = if handles_urls {
        "MimeType=x-scheme-handler/codexplusplus;\n"
    } else {
        ""
    };
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment={comment}\nExec={}{}\nIcon={ICON_NAME}\nTerminal=false\nCategories=Development;\nStartupNotify=true\n{mime_type}",
        desktop_exec_path(executable),
        url_argument,
    )
}

fn desktop_exec_path(path: &std::path::Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    format!("\"{escaped}\"")
}

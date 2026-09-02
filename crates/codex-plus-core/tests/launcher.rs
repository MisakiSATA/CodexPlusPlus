use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codex_plus_core::app_paths::{
    build_codex_executable, codex_app_version, find_latest_codex_app_dir,
    find_latest_codex_app_dir_from_roots, find_macos_codex_app, normalize_codex_app_path,
    packaged_app_user_model_id, resolve_codex_app_dir_with_saved, user_data_candidates_from,
};
#[cfg(target_os = "linux")]
use codex_plus_core::app_paths::{find_linux_codex_app, find_linux_codex_app_default_from};
use codex_plus_core::launcher::{
    CodexLaunch, DefaultLaunchHooks, LaunchHooks, LaunchOptions, MacosCleanupPolicy,
    browser_identity_changed, build_codex_arguments, build_codex_arguments_for_settings,
    build_codex_arguments_with_native_menu_inspector, build_codex_command,
    build_codex_command_with_native_menu_inspector, build_macos_cleanup_command,
    build_macos_open_command, build_macos_open_command_with_native_menu_inspector,
    build_packaged_activation, build_packaged_activation_with_native_menu_inspector,
    launch_and_inject_with_hooks,
};
#[cfg(windows)]
use codex_plus_core::launcher::{WindowsProcessControlStrategy, windows_process_control_strategy};
use codex_plus_core::ports::{
    select_packaged_codex_debug_port_with, select_platform_loopback_port_with,
};
use codex_plus_core::relay_switch::acquire_relay_switch_lock;
use codex_plus_core::settings::{BackendSettings, RelayProfile, RelayProtocol, SettingsStore};
use codex_plus_core::status::StatusStore;

#[test]
fn browser_identity_change_requires_two_distinct_observations() {
    assert!(!browser_identity_changed(None, "browser-a"));
    assert!(!browser_identity_changed(Some("browser-a"), "browser-a"));
    assert!(browser_identity_changed(Some("browser-a"), "browser-b"));
}

#[test]
fn app_paths_find_latest_windows_package_prefers_highest_version_app_dir() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_1.2.3.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_not-a-version_x64__abc")).unwrap();

    let latest = find_latest_codex_app_dir(temp.path()).unwrap();

    assert_eq!(
        latest,
        temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc/app")
    );
}

#[test]
fn app_paths_find_latest_windows_package_ignores_chatgpt_desktop_package() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("OpenAI.Codex_26.707.3748.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(
        temp.path()
            .join("OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc/app"),
    )
    .unwrap();
    std::fs::create_dir_all(
        temp.path()
            .join("OpenAI.ChatGPT-Desktop_2026.514.421.0_neutral_~_abc"),
    )
    .unwrap();

    let latest = find_latest_codex_app_dir(temp.path()).unwrap();

    assert_eq!(
        latest,
        temp.path().join("OpenAI.Codex_26.707.3748.0_x64__abc/app")
    );
    assert_eq!(codex_app_version(&latest).as_deref(), Some("26.707.3748.0"));
    assert_eq!(
        packaged_app_user_model_id(&latest).as_deref(),
        Some("OpenAI.Codex_abc!App")
    );
}

#[test]
fn app_paths_find_latest_windows_package_detects_beta_package() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(
        temp.path()
            .join("OpenAI.CodexBeta_26.527.7698.0_x64__2p2nqsd0c76g0/app"),
    )
    .unwrap();

    let latest = find_latest_codex_app_dir(temp.path()).unwrap();

    assert_eq!(
        latest,
        temp.path()
            .join("OpenAI.CodexBeta_26.527.7698.0_x64__2p2nqsd0c76g0/app")
    );
    assert_eq!(codex_app_version(&latest).as_deref(), Some("26.527.7698.0"));
    assert_eq!(
        packaged_app_user_model_id(&latest).as_deref(),
        Some("OpenAI.CodexBeta_2p2nqsd0c76g0!App")
    );
}

#[test]
fn app_paths_find_latest_windows_package_returns_package_when_app_dir_missing() {
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("OpenAI.Codex_26.429.8261.0_x64__abc");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("ChatGPT.exe"), "").unwrap();

    assert_eq!(find_latest_codex_app_dir(temp.path()).unwrap(), package);
}

#[test]
fn app_paths_find_latest_windows_package_checks_roots_before_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("WindowsApps");
    std::fs::create_dir_all(root.join("OpenAI.Codex_1.0.0.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(root.join("OpenAI.Codex_26.513.3673.0_x64__abc/app")).unwrap();

    let latest = find_latest_codex_app_dir_from_roots(&[root]).unwrap();

    assert!(latest.ends_with("OpenAI.Codex_26.513.3673.0_x64__abc/app"));
}

#[test]
fn app_paths_find_latest_windows_package_ignores_chatgpt_across_roots() {
    let temp = tempfile::tempdir().unwrap();
    let root_a = temp.path().join("WindowsAppsA");
    let root_b = temp.path().join("WindowsAppsB");
    std::fs::create_dir_all(root_a.join("OpenAI.Codex_26.999.0.0_x64__abc/app")).unwrap();
    std::fs::create_dir_all(root_b.join("OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc/app"))
        .unwrap();

    let latest = find_latest_codex_app_dir_from_roots(&[root_a, root_b]).unwrap();

    assert!(latest.ends_with("OpenAI.Codex_26.999.0.0_x64__abc/app"));
}

#[test]
fn app_paths_extracts_codex_version_from_windows_package_app_dir() {
    let app_dir =
        PathBuf::from(r"C:\Program Files\WindowsApps\OpenAI.Codex_26.513.3673.0_x64__abc\app");

    assert_eq!(
        codex_app_version(&app_dir).as_deref(),
        Some("26.513.3673.0")
    );
}

#[test]
fn app_paths_extracts_codex_version_from_portable_version_file() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("versions").join("current");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("Codex.exe"), "").unwrap();
    std::fs::write(app_dir.join("version"), "42.1.0\n").unwrap();

    assert_eq!(codex_app_version(&app_dir).as_deref(), Some("42.1.0"));
}

#[test]
fn app_paths_prefers_portable_directory_version_over_internal_version_file() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("versions").join("26.519.2736.0");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("Codex.exe"), "").unwrap();
    std::fs::write(app_dir.join("version"), "42.1.0\n").unwrap();

    assert_eq!(
        codex_app_version(&app_dir).as_deref(),
        Some("26.519.2736.0")
    );
}

#[cfg(windows)]
#[test]
fn app_paths_resolves_portable_current_link_to_directory_version() {
    let temp = tempfile::tempdir().unwrap();
    let versions = temp.path().join("versions");
    let target = versions.join("26.519.2736.0");
    let current = versions.join("current");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("Codex.exe"), "").unwrap();
    std::fs::write(target.join("version"), "42.1.0\n").unwrap();
    std::os::windows::fs::symlink_dir(&target, &current).unwrap();

    assert_eq!(
        codex_app_version(&current).as_deref(),
        Some("26.519.2736.0")
    );
}

#[test]
fn app_paths_extracts_codex_version_from_macos_bundle_plist() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("OpenAI Codex.app");
    let contents = app.join("Contents");
    std::fs::create_dir_all(&contents).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleVersion</key>
  <string>26.500.0</string>
  <key>CFBundleShortVersionString</key>
  <string>26.513.3673</string>
</dict>
</plist>
"#,
    )
    .unwrap();

    assert_eq!(codex_app_version(&app).as_deref(), Some("26.513.3673"));
}

#[test]
fn app_paths_user_data_candidates_include_local_and_roaming_variants() {
    let local = PathBuf::from(r"C:\Users\me\AppData\Local");
    let roaming = PathBuf::from(r"C:\Users\me\AppData\Roaming");

    let candidates = user_data_candidates_from(Some(&local), Some(&roaming));

    assert_eq!(
        candidates,
        vec![
            local.join("OpenAI").join("ChatGPT"),
            local.join("OpenAI.ChatGPT-Desktop"),
            local.join("ChatGPT"),
            local.join("OpenAI").join("Codex"),
            local.join("OpenAI.Codex"),
            local.join("Codex"),
            roaming.join("OpenAI").join("ChatGPT"),
            roaming.join("OpenAI.ChatGPT-Desktop"),
            roaming.join("ChatGPT"),
            roaming.join("OpenAI").join("Codex"),
            roaming.join("OpenAI.Codex"),
            roaming.join("Codex"),
        ]
    );
}

#[test]
fn app_paths_find_macos_codex_app_prefers_first_search_root_and_known_names() {
    let temp = tempfile::tempdir().unwrap();
    let system_root = temp.path().join("Applications");
    let user_root = temp.path().join("Users/me/Applications");
    let system_app = system_root.join("OpenAI Codex.app");
    let user_app = user_root.join("Codex.app");
    std::fs::create_dir_all(&system_app).unwrap();
    std::fs::create_dir_all(&user_app).unwrap();

    assert_eq!(
        find_macos_codex_app(&[system_root, user_root]).unwrap(),
        system_app
    );
}

#[test]
fn app_paths_prefers_codex_app_over_chatgpt_app() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Applications");
    let codex = root.join("Codex.app");
    let chatgpt = root.join("ChatGPT.app");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(&chatgpt).unwrap();

    assert_eq!(
        find_macos_codex_app(&[root]).as_deref(),
        Some(codex.as_path())
    );
}

#[test]
fn app_paths_preserves_legacy_macos_candidates_before_chatgpt_app() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Applications");
    let legacy = root.join("OpenAI Codex.app");
    let chatgpt = root.join("ChatGPT.app");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&chatgpt).unwrap();

    assert_eq!(
        find_macos_codex_app(&[root]).as_deref(),
        Some(legacy.as_path())
    );
}

#[test]
fn app_paths_build_macos_bundle_executable() {
    let app = PathBuf::from("/Applications/OpenAI Codex.app");

    assert_eq!(
        build_codex_executable(&app),
        PathBuf::from("/Applications/OpenAI Codex.app/Contents/MacOS/Codex")
    );
}

#[test]
fn app_paths_finds_chatgpt_bundle_and_uses_its_declared_executable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Applications");
    let app = root.join("ChatGPT.app");
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.openai.codex</string>
  <key>CFBundleExecutable</key>
  <string>ChatGPT</string>
</dict>
</plist>
"#,
    )
    .unwrap();
    std::fs::write(macos.join("ChatGPT"), "").unwrap();

    assert_eq!(
        find_macos_codex_app(&[root]).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(build_codex_executable(&app), macos.join("ChatGPT"));
}

#[test]
fn app_paths_normalizes_executable_and_package_paths() {
    let temp = tempfile::tempdir().unwrap();
    let portable = temp.path().join("CodexPortable");
    let app = portable.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("Codex.exe"), "").unwrap();

    assert_eq!(
        normalize_codex_app_path(&app.join("Codex.exe")).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(
        normalize_codex_app_path(&portable).as_deref(),
        Some(app.as_path())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn app_paths_normalizes_linux_aur_wrapper_executable() {
    let temp = tempfile::tempdir().unwrap();
    let portable = temp.path().join("codex-plus-plus");
    let app = portable.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("codex"), "").unwrap();

    assert_eq!(
        normalize_codex_app_path(&app.join("codex")).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(
        normalize_codex_app_path(&portable).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(
        find_linux_codex_app(&[portable]).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(build_codex_executable(&app), app.join("codex"));
}

#[cfg(target_os = "linux")]
#[test]
fn app_paths_prefers_current_linux_chatgpt_package_layout() {
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path().join("chatgpt");
    let previous = temp.path().join("openai-codex-desktop");
    let legacy = temp.path().join("codex-plus-plus/app");
    for app in [&current, &previous, &legacy] {
        std::fs::create_dir_all(app).unwrap();
        std::fs::write(app.join("ChatGPT"), "").unwrap();
        if app != &legacy {
            std::fs::create_dir_all(app.join("resources")).unwrap();
            std::fs::write(app.join("resources/app.asar"), "").unwrap();
        }
    }

    assert_eq!(
        find_linux_codex_app_default_from(&[current.clone(), previous.clone(), legacy.clone(),])
            .as_deref(),
        Some(current.as_path())
    );

    std::fs::remove_file(current.join("ChatGPT")).unwrap();
    assert_eq!(
        find_linux_codex_app_default_from(&[current, previous.clone(), legacy]).as_deref(),
        Some(previous.as_path())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn app_paths_does_not_treat_the_linux_codex_cli_as_desktop() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("codex"), "").unwrap();

    assert_eq!(normalize_codex_app_path(&bin.join("codex")), None);
    assert_eq!(normalize_codex_app_path(&bin), None);
}

#[test]
fn app_paths_prefers_chatgpt_entrypoint_when_portable_bundle_contains_codex_shim() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("current");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("Codex.exe"), "").unwrap();
    std::fs::write(app.join("ChatGPT.exe"), "").unwrap();

    assert_eq!(build_codex_executable(&app), app.join("ChatGPT.exe"));
}

#[test]
fn app_paths_normalizes_chatgpt_desktop_executable_and_builds_it() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp
        .path()
        .join("OpenAI.Codex_1.2026.133.0_x64__abc")
        .join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("ChatGPT.exe"), "").unwrap();

    assert_eq!(
        normalize_codex_app_path(&app.join("ChatGPT.exe")).as_deref(),
        Some(app.as_path())
    );
    assert_eq!(build_codex_executable(&app), app.join("ChatGPT.exe"));
    assert_eq!(
        packaged_app_user_model_id(&app).as_deref(),
        Some("OpenAI.Codex_abc!App")
    );
}

#[test]
fn app_paths_saved_path_is_used_when_no_explicit_path_is_provided() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app).unwrap();

    assert_eq!(
        resolve_codex_app_dir_with_saved(None, Some(&app.to_string_lossy())).as_deref(),
        Some(app.as_path())
    );
}

#[test]
fn app_paths_rejects_codex_plus_plus_install_dir_as_codex_app() {
    let temp = tempfile::tempdir().unwrap();
    let manager = temp.path().join("Programs").join("Codex++");
    std::fs::create_dir_all(&manager).unwrap();
    std::fs::write(manager.join("Codex++ Manager.exe"), "").unwrap();

    assert_eq!(normalize_codex_app_path(&manager), None);
    assert_eq!(
        normalize_codex_app_path(&manager.join("Codex++ Manager.exe")),
        None
    );

    let resolved = resolve_codex_app_dir_with_saved(None, Some(&manager.to_string_lossy()));
    assert_ne!(resolved.as_deref(), Some(manager.as_path()));
}

#[test]
fn app_paths_rejects_plain_directory_without_codex_executable() {
    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("not-a-codex-app");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("readme.txt"), "nope").unwrap();

    assert_eq!(normalize_codex_app_path(&plain), None);
    assert_eq!(normalize_codex_app_path(&plain.join("readme.txt")), None);
}

#[test]
fn app_paths_empty_saved_path_matches_no_saved_path() {
    assert_eq!(
        resolve_codex_app_dir_with_saved(None, Some("")),
        resolve_codex_app_dir_with_saved(None, None)
    );
    assert_eq!(
        resolve_codex_app_dir_with_saved(None, Some("   ")),
        resolve_codex_app_dir_with_saved(None, None)
    );
}

#[test]
fn app_paths_invalid_saved_path_falls_back_instead_of_sticking() {
    let temp = tempfile::tempdir().unwrap();
    let junk = temp.path().join("Codex++");
    std::fs::create_dir_all(&junk).unwrap();

    // 合法独立安装：即使 saved 指向 Codex++，规范化失败后应能落到该候选
    // （通过显式 app_dir 验证回退链之外的合法路径仍可用）
    let standalone = temp.path().join("OpenAI").join("Codex").join("bin");
    std::fs::create_dir_all(&standalone).unwrap();
    std::fs::write(standalone.join("codex.exe"), "").unwrap();

    assert_eq!(normalize_codex_app_path(&junk), None);
    assert_eq!(
        normalize_codex_app_path(&standalone).as_deref(),
        Some(standalone.as_path())
    );
    assert_eq!(
        resolve_codex_app_dir_with_saved(Some(&standalone), Some(&junk.to_string_lossy()))
            .as_deref(),
        Some(standalone.as_path())
    );
}

#[test]
fn launcher_builds_debug_arguments_and_commands() {
    let app_dir = PathBuf::from(r"C:\Codex\app");

    assert_eq!(
        build_codex_arguments(9229, &[]),
        vec![
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
        ]
    );
    let command = build_codex_command(&app_dir, 9229, &[]);
    assert_eq!(command[1], "--remote-debugging-port=9229");
    assert_eq!(command[2], "--remote-allow-origins=http://127.0.0.1:9229");
}

#[test]
fn launcher_does_not_override_codex_app_environment() {
    let source = include_str!("../src/launcher.rs");

    assert!(!source.contains(".envs(codex_process_environment())"));
    assert!(!source.contains("activate_packaged_app_with_environment"));
    assert!(!source.contains("with_temporary_proxy_environment"));
}

#[test]
fn launcher_prepares_projectless_main_window_when_enhancements_are_enabled() {
    let source = include_str!("../src/launcher.rs");

    assert!(source.contains("if settings.enhancements_enabled"));
    assert!(source.contains("prepare_projectless_main_window_nonfatal"));
    assert!(source.contains("launcher.prelaunch"));
}

#[test]
fn launcher_windows_process_wait_uses_platform_cfg_guards() {
    let source = include_str!("../src/launcher.rs").replace("\r\n", "\n");

    assert!(source.contains(
        "#[cfg(windows)]\nasync fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()>"
    ));
    assert!(source.contains(
        "#[cfg(not(windows))]\nasync fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()>"
    ));
    assert!(source.contains(
        "#[cfg(windows)]\nfn wait_for_windows_process_id_blocking(process_id: u32) -> anyhow::Result<()>"
    ));
}

#[test]
fn launcher_appends_extra_codex_arguments_after_debug_arguments() {
    let app_dir = PathBuf::from(r"C:\Codex\app");
    let extra_args = vec![
        "--force_high_performance_gpu".to_string(),
        "  ".to_string(),
        "--enable-features=UseOzonePlatform".to_string(),
    ];

    assert_eq!(
        build_codex_arguments(9229, &extra_args),
        vec![
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
            "--force_high_performance_gpu".to_string(),
            "--enable-features=UseOzonePlatform".to_string(),
        ]
    );
    let command = build_codex_command(&app_dir, 9229, &extra_args);
    assert_eq!(command[1], "--remote-debugging-port=9229");
    assert_eq!(command[2], "--remote-allow-origins=http://127.0.0.1:9229");
    assert_eq!(command[3], "--force_high_performance_gpu");
    assert_eq!(command[4], "--enable-features=UseOzonePlatform");
}

#[test]
fn launcher_fast_startup_adds_statsig_fast_fail_argument_when_enabled() {
    let settings = BackendSettings {
        codex_app_fast_startup: true,
        ..BackendSettings::default()
    };
    let args = build_codex_arguments_for_settings(9229, &settings);

    assert!(args.iter().any(|arg| {
        arg.starts_with("--host-resolver-rules=")
            && arg.contains("MAP ab.chatgpt.com 127.0.0.1")
            && arg.contains("MAP featureassets.org 127.0.0.1")
            && arg.contains("MAP cloudflare-dns.com 127.0.0.1")
    }));

    let settings = BackendSettings {
        codex_app_fast_startup: true,
        codex_extra_args: vec!["--host-resolver-rules=MAP example.test 127.0.0.1".to_string()],
        ..BackendSettings::default()
    };
    let args = build_codex_arguments_for_settings(9229, &settings);
    assert_eq!(
        args.iter()
            .filter(|arg| arg.starts_with("--host-resolver-rules="))
            .count(),
        1
    );

    let settings = BackendSettings {
        codex_app_fast_startup: false,
        ..BackendSettings::default()
    };
    let args = build_codex_arguments_for_settings(9229, &settings);
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("--host-resolver-rules="))
    );
}

#[test]
fn launcher_native_menu_inspector_arguments_are_added_before_extra_args() {
    let app_dir = PathBuf::from(r"C:\Codex\app");
    let extra_args = vec!["--force_high_performance_gpu".to_string()];

    assert_eq!(
        build_codex_arguments_with_native_menu_inspector(9229, 9329, &extra_args),
        vec![
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
            "--inspect=127.0.0.1:9329".to_string(),
            "--force_high_performance_gpu".to_string(),
        ]
    );
    let command = build_codex_command_with_native_menu_inspector(&app_dir, 9229, 9329, &extra_args);
    assert_eq!(command[1], "--remote-debugging-port=9229");
    assert_eq!(command[2], "--remote-allow-origins=http://127.0.0.1:9229");
    assert_eq!(command[3], "--inspect=127.0.0.1:9329");
    assert_eq!(command[4], "--force_high_performance_gpu");
}

#[test]
fn launcher_constructs_windows_packaged_activation_without_real_app() {
    let app_dir = PathBuf::from(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0\app",
    );

    assert_eq!(
        packaged_app_user_model_id(&app_dir).unwrap(),
        "OpenAI.Codex_2p2nqsd0c76g0!App"
    );
    assert_eq!(
        build_packaged_activation(&app_dir, 9229, &[]).unwrap(),
        CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229"
                .to_string(),
            process_id: None,
        }
    );
}

#[test]
fn launcher_packaged_activation_appends_extra_codex_arguments() {
    let app_dir = PathBuf::from(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0\app",
    );
    let extra_args = vec!["--force_high_performance_gpu".to_string()];

    assert_eq!(
        build_packaged_activation(&app_dir, 9229, &extra_args).unwrap(),
        CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments:
                "--remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229 --force_high_performance_gpu"
                    .to_string(),
            process_id: None,
        }
    );
}

#[test]
fn launcher_packaged_activation_adds_native_menu_inspector_argument() {
    let app_dir = PathBuf::from(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.506.2212.0_x64__2p2nqsd0c76g0\app",
    );

    assert_eq!(
        build_packaged_activation_with_native_menu_inspector(&app_dir, 9229, 9329, &[]).unwrap(),
        CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments:
                "--remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229 --inspect=127.0.0.1:9329"
                    .to_string(),
            process_id: None,
        }
    );
}

#[test]
fn launcher_packaged_activation_can_preserve_process_id() {
    let launch = CodexLaunch::PackagedActivation {
        app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
        arguments: "--remote-debugging-port=9229".to_string(),
        process_id: Some(4242),
    };

    assert_eq!(launch.process_id(), Some(4242));
}

#[test]
fn launcher_packaged_cleanup_requires_a_known_ownership_baseline() {
    use codex_plus_core::launcher::{
        PackagedProcessCleanupAction, WindowsProcessIdentity, packaged_process_cleanup_action,
    };

    let existing = WindowsProcessIdentity::new(4242, 100);
    let reused = WindowsProcessIdentity::new(4242, 200);

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
    assert_eq!(
        packaged_process_cleanup_action(Some(&[existing]), None),
        PackagedProcessCleanupAction::WaitForExitWithoutTermination
    );
}

#[test]
fn launcher_applies_codexplusplus_window_icon_after_packaged_activation() {
    let source = include_str!("../src/launcher.rs");

    assert!(source.contains("apply_codexplusplus_window_icon_after_launch(process_id);"));
    assert!(source.contains("windows_apply_codexplusplus_icon_to_process_window"));
}

#[test]
fn launcher_no_longer_contains_mobile_control_runtime() {
    let launcher_source = include_str!("../src/launcher.rs");
    let settings_source = include_str!("../src/settings.rs");
    let workspace_toml = include_str!("../../../Cargo.toml");

    assert!(!workspace_toml.contains("apps/codex-plus-mobile-relay"));
    assert!(!launcher_source.contains("MobileRelay"));
    assert!(!launcher_source.contains("mobile_relay"));
    assert!(!launcher_source.contains("\"/mobile\""));
    assert!(!launcher_source.contains("CODEX_PLUS_MOBILE"));
    assert!(!settings_source.contains("mobileControl"));
}

#[test]
fn launcher_plugin_marketplace_config_reuses_existing_relay_switch_lock() {
    let launcher_source = include_str!("../src/launcher.rs");

    assert!(launcher_source.contains("ensure_openai_curated_marketplace_config_with_lock("));
    assert!(launcher_source.contains("ensure_role_specific_plugins_marketplace_config_with_lock("));
    assert!(
        launcher_source.contains("relay_switch_lock: &crate::relay_switch::RelaySwitchLockGuard")
    );
}

#[test]
fn app_paths_uses_native_windows_package_api_without_powershell() {
    let source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app_paths.rs")).unwrap();

    assert!(source.contains("GetPackagesByPackageFamily"));
    assert!(source.contains("GetPackagePathByFullName"));
    assert!(!source.contains("Command::new(\"powershell\")"));
}

#[test]
fn windows_process_enumeration_preserves_snapshot_failures() {
    let source = include_str!("../src/windows_integration.rs");

    assert!(
        source.contains(
            "pub fn try_enumerate_processes() -> anyhow::Result<Vec<WindowsProcessInfo>>"
        )
    );
    assert!(source.contains("ERROR_NO_MORE_FILES"));
    assert!(source.contains("try_enumerate_processes().unwrap_or_default()"));
}

#[test]
fn launcher_packaged_activation_does_not_directly_fallback_to_windowsapps_exe() {
    let source = include_str!("../src/launcher.rs");
    let hooks_impl_start = source
        .find("impl LaunchHooks for DefaultLaunchHooks")
        .expect("default launch hooks implementation should exist");
    let launch_codex_start = source[hooks_impl_start..]
        .find("async fn launch_codex")
        .map(|offset| hooks_impl_start + offset)
        .expect("default launch hooks should implement launch_codex");
    let launch_codex_end = source[launch_codex_start..]
        .find("async fn inject")
        .map(|offset| launch_codex_start + offset)
        .expect("launch_codex should be followed by inject");
    let launch_codex = &source[launch_codex_start..launch_codex_end];

    assert!(!launch_codex.contains("launcher.packaged_activation_cdp_unready_direct_fallback"));
    assert!(!launch_codex.contains("terminate_windows_process_id(process_id).await"));
}

#[test]
fn launcher_packaged_activation_requires_a_stable_identity_baseline() {
    use codex_plus_core::launcher::{
        PackagedProcessCleanupAction, WindowsProcessIdentity, packaged_process_cleanup_action,
    };

    let baseline = WindowsProcessIdentity::new(4242, 100);
    assert_eq!(
        packaged_process_cleanup_action(Some(&[baseline]), None),
        PackagedProcessCleanupAction::WaitForExitWithoutTermination
    );
    assert_eq!(
        packaged_process_cleanup_action(
            Some(&[baseline]),
            Some(WindowsProcessIdentity::new(4242, 200)),
        ),
        PackagedProcessCleanupAction::WaitForExitWithoutTermination
    );
}

#[test]
fn launcher_windows_descendant_snapshot_fails_closed() {
    let source = include_str!("../src/launcher.rs");
    let cleanup_start = source
        .find("async fn cleanup_owned_windows_process")
        .expect("Windows owned cleanup helper should exist");
    let cleanup_end = source[cleanup_start..]
        .find("#[cfg(target_os = \"linux\")]")
        .map(|offset| cleanup_start + offset)
        .expect("Windows owned cleanup should be followed by Linux cleanup");
    let cleanup = &source[cleanup_start..cleanup_end];

    assert!(source.contains("try_capture_windows_descendant_handles"));
    assert!(source.contains("validate_windows_descendant_identities"));
    assert!(!source.contains("terminate_windows_process_ids"));
    assert_eq!(
        cleanup.matches("terminate_windows_process_handle").count(),
        1
    );
    assert!(cleanup.contains("wait_for_windows_process_handle_confirmed(handle).await"));
}

#[test]
fn launcher_windows_termination_accepts_an_already_exited_process() {
    let source = include_str!("../src/launcher.rs");
    let terminate_start = source
        .find("fn terminate_windows_process_handle_blocking")
        .expect("Windows blocking termination helper should exist");
    let terminate_end = source[terminate_start..]
        .find("#[cfg(not(windows))]")
        .map(|offset| terminate_start + offset)
        .expect("Windows helper should be followed by a non-Windows fallback");
    let terminate = &source[terminate_start..terminate_end];

    assert!(terminate.contains("WaitForSingleObject(handle.raw_handle(), 0)"));
    assert!(terminate.contains("WAIT_OBJECT_0"));
    assert!(!terminate.contains("OpenProcess"));
}

#[test]
fn launcher_windows_termination_never_reopens_a_raw_pid() {
    let source = include_str!("../src/launcher.rs");

    assert!(!source.contains("terminate_windows_process_id("));
    assert!(!source.contains("terminate_windows_process_ids("));
    assert!(source.contains("terminate_windows_process_handle("));
}

#[cfg(windows)]
#[test]
fn launcher_windows_packaged_process_management_uses_native_api() {
    assert_eq!(
        windows_process_control_strategy(),
        WindowsProcessControlStrategy::NativeWindowsApi
    );
}

#[test]
fn launcher_macos_open_command_waits_for_app_exit() {
    let command = build_macos_open_command(Path::new("/Applications/Codex.app"), 9229, &[]);

    assert_eq!(command[0], "open");
    assert!(command.contains(&"-W".to_string()));
    assert!(command.contains(&"-a".to_string()));
    assert!(command.contains(&"--args".to_string()));
    assert!(command.contains(&"--remote-debugging-port=9229".to_string()));
}

#[test]
fn launcher_macos_open_command_appends_extra_codex_arguments_after_args() {
    let extra_args = vec!["--force_high_performance_gpu".to_string()];
    let command = build_macos_open_command(Path::new("/Applications/Codex.app"), 9229, &extra_args);
    let args_index = command
        .iter()
        .position(|part| part == "--args")
        .expect("macOS command should contain --args");

    assert_eq!(
        &command[args_index + 1..],
        &[
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
            "--force_high_performance_gpu".to_string(),
        ]
    );
}

#[test]
fn launcher_macos_open_command_adds_native_menu_inspector_argument() {
    let command = build_macos_open_command_with_native_menu_inspector(
        Path::new("/Applications/Codex.app"),
        9229,
        9329,
        &[],
    );
    let args_index = command
        .iter()
        .position(|part| part == "--args")
        .expect("macOS command should contain --args");

    assert_eq!(
        &command[args_index + 1..],
        &[
            "--remote-debugging-port=9229".to_string(),
            "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
            "--inspect=127.0.0.1:9329".to_string(),
        ]
    );
}

#[test]
fn ports_falls_back_to_ephemeral_when_requested_is_busy() {
    let selected = select_platform_loopback_port_with(9229, true, |_| false, || 43001);

    assert_eq!(selected, 43001);
}

#[test]
fn ports_windows_packaged_debug_falls_back_to_ephemeral_when_requested_is_busy() {
    let selected = select_packaged_codex_debug_port_with(9229, true, |_| false, || 43001);

    assert_eq!(selected, 43001);
}

#[test]
fn ports_keeps_requested_when_fallback_disabled() {
    let selected = select_platform_loopback_port_with(9229, false, |_| false, || 43001);

    assert_eq!(selected, 9229);
}

#[tokio::test]
async fn default_helper_serves_backend_status_over_http() {
    let hooks = DefaultLaunchHooks::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    hooks.start_helper(port).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(format!("http://127.0.0.1:{port}/backend/status"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let payload: serde_json::Value = response.json().await.unwrap();
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["transport"], "http-helper");

    let repair_response = client
        .post(format!("http://127.0.0.1:{port}/backend/repair"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert!(!repair_response.status().is_success());

    hooks.shutdown_helper(port).await;
}

#[tokio::test]
async fn default_helper_accepts_diagnostic_log_events_over_http() {
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("codex-plus.log");
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path.clone()));
    let hooks = DefaultLaunchHooks::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    hooks.start_helper(port).await.unwrap();
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}/diagnostics/log"))
        .json(&serde_json::json!({
            "event": "backend_check_failed",
            "message": "fetch failed",
            "helperBase": format!("http://127.0.0.1:{port}")
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let payload: serde_json::Value = response.json().await.unwrap();
    assert_eq!(payload["status"], "ok");
    hooks.shutdown_helper(port).await;

    let contents = std::fs::read_to_string(&log_path).unwrap();
    assert!(contents.contains("renderer.backend_check_failed"));
    assert!(contents.contains("fetch failed"));
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(None);
}

#[tokio::test]
async fn launch_releases_relay_switch_lock_once_page_confirms_config_load() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            enhancements_enabled: true,
            relay_profiles_enabled: true,
            ..BackendSettings::default()
        })
        .with_relay_lock_probe_at_injection();

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(
        events.contains(&"injection-relay-lock:free".to_string()),
        "页面就绪后注入阶段切换锁必须已释放，实际事件：{events:?}"
    );
}

#[test]
fn launcher_startup_injection_retry_is_time_bounded() {
    let source = include_str!("../src/launcher.rs");
    let start = source
        .find("async fn ensure_injection")
        .expect("default ensure_injection should exist");
    let end = source[start..]
        .find("async fn start_bridge_watchdog")
        .map(|offset| start + offset)
        .expect("ensure_injection should be followed by start_bridge_watchdog");
    let section = &source[start..end];

    // 注入重试必须按总时长封顶：CDP 一直不可达时不能拖住失败清理和切换锁几十分钟。
    assert!(section.contains("STARTUP_INJECTION_RETRY_WINDOW"));
    assert!(!section.contains("1..=120"));
}

#[tokio::test]
async fn launch_lifecycle_runs_enabled_maintenance_without_applying_relay_profile() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            provider_sync_enabled: true,
            relay_profiles_enabled: true,
            computer_use_guard_enabled: true,
            codex_app_plugin_marketplace_unlock: true,
            ..BackendSettings::default()
        })
        .with_launch_result(CodexLaunch::Process {
            command: vec!["codex".to_string()],
            wait_strategy: codex_plus_core::launcher::ProcessWaitStrategy::TrackedChild,
            macos_cleanup_policy: None,
        });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir.clone()),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "provider-sync",
            "computer-use-guard",
            "start-helper:57321",
            "launch:9229",
            "computer-use-guard-watchdog",
            "inject:9229:57321",
            "status:running",
            "wait-codex",
            "shutdown-helper:57321",
        ]
    );
    let events = events.lock().unwrap().clone();
    assert!(!events.contains(&"apply-relay".to_string()));
    assert!(events.contains(&"provider-sync".to_string()));
    assert!(events.contains(&"computer-use-guard".to_string()));
    assert!(events.contains(&"computer-use-guard-watchdog".to_string()));
    assert_eq!(
        handle
            .status_store
            .load_latest()
            .unwrap()
            .unwrap()
            .codex_app
            .as_deref(),
        Some(app_dir.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn launch_lifecycle_passes_configured_extra_args_to_codex_launch() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        codex_extra_args: vec!["--force_high_performance_gpu".to_string()],
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert!(
        events
            .lock()
            .unwrap()
            .contains(&"launch:9229:--force_high_performance_gpu".to_string())
    );
}

#[tokio::test]
async fn launch_lifecycle_passes_native_menu_localization_switch_to_codex_launch() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        codex_app_native_menu_localization: false,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert!(
        events
            .lock()
            .unwrap()
            .contains(&"launch:9229:native-menu-off".to_string())
    );
}

#[tokio::test]
async fn launch_lifecycle_keeps_js_injection_in_relay_mode() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        launch_mode: codex_plus_core::settings::LaunchMode::Relay,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "status:running",
            "wait-codex",
            "shutdown-helper:57321",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_skips_helper_and_injection_when_enhancements_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "launch:9229",
            "status:running",
            "wait-codex",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_runs_computer_use_guard_when_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        computer_use_guard_enabled: true,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "computer-use-guard",
            "start-helper:57321",
            "launch:9229",
            "computer-use-guard-watchdog",
            "inject:9229:57321",
            "status:running",
            "wait-codex",
            "shutdown-helper:57321",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_skips_computer_use_guard_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone());

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(!events.contains(&"computer-use-guard".to_string()));
    assert!(!events.contains(&"computer-use-guard-watchdog".to_string()));
    assert!(events.contains(&"launch:9229".to_string()));
}

#[test]
fn launch_lifecycle_waits_for_relay_switch_lock_before_loading_settings() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone());
    let lock = acquire_relay_switch_lock(hooks.codex_home.path()).unwrap();
    let thread_hooks = hooks.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let launcher = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        started_tx.send(()).unwrap();
        runtime.block_on(async move {
            let handle = launch_and_inject_with_hooks(
                LaunchOptions {
                    app_dir: Some(app_dir),
                    debug_port: 9229,
                    helper_port: 57321,
                    status_store: StatusStore::new(temp.path().join("latest-status.json")),
                },
                &thread_hooks,
            )
            .await?;
            handle.wait_for_codex_exit().await
        })
    });

    started_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !events
            .lock()
            .unwrap()
            .contains(&"load-settings".to_string())
    );
    drop(lock);
    launcher.join().unwrap().unwrap();
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&"load-settings".to_string())
    );
}

#[tokio::test]
async fn launch_lifecycle_does_not_apply_official_relay_profile_before_launching_codex() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        relay_profiles_enabled: true,
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(!events.contains(&"apply-relay".to_string()));
    assert!(events.contains(&"launch:9229".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_ignores_stale_chat_protocol_on_official_relay_profile() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        relay_profiles_enabled: true,
        relay_profiles: vec![RelayProfile {
            id: "official".to_string(),
            name: "Official".to_string(),
            protocol: RelayProtocol::ChatCompletions,
            relay_mode: codex_plus_core::settings::RelayMode::Official,
            ..RelayProfile::default()
        }],
        active_relay_id: "official".to_string(),
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(!events.contains(&"apply-relay".to_string()));
    assert!(
        !events
            .iter()
            .any(|event| event.starts_with("start-helper:"))
    );
    assert!(events.contains(&"launch:9229".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_applies_official_mix_chat_profile_and_starts_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        relay_profiles_enabled: true,
        relay_profiles: vec![RelayProfile {
            id: "official-mix".to_string(),
            name: "Official Mix".to_string(),
            protocol: RelayProtocol::ChatCompletions,
            relay_mode: codex_plus_core::settings::RelayMode::Official,
            official_mix_api_key: true,
            ..RelayProfile::default()
        }],
        active_relay_id: "official-mix".to_string(),
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 58000,
            status_store: StatusStore::new(temp.path().join("latest-status.json")),
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(events.contains(&"apply-relay".to_string()));
    assert!(events.contains(&"start-helper:57321".to_string()));
    assert!(!events.contains(&"inject:9229:57321".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_applies_protocol_proxy_relay_before_provider_sync_and_launch() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        provider_sync_enabled: true,
        relay_profiles_enabled: true,
        relay_profiles: vec![RelayProfile {
            id: "relay-chat".to_string(),
            name: "Chat Completions".to_string(),
            protocol: RelayProtocol::ChatCompletions,
            relay_mode: codex_plus_core::settings::RelayMode::MixedApi,
            ..RelayProfile::default()
        }],
        active_relay_id: "relay-chat".to_string(),
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    let load_index = events
        .iter()
        .position(|event| event == "load-settings")
        .unwrap();
    let apply_index = events
        .iter()
        .position(|event| event == "apply-relay")
        .unwrap();
    let sync_index = events
        .iter()
        .position(|event| event == "provider-sync")
        .unwrap();
    let launch_index = events
        .iter()
        .position(|event| event == "launch:9229")
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "apply-relay")
            .count(),
        1
    );
    assert!(load_index < apply_index);
    assert!(apply_index < sync_index);
    assert!(sync_index < launch_index);
    assert_eq!(
        hooks.applied_relay_home.lock().unwrap().as_deref(),
        Some(hooks.codex_home.path())
    );
    assert_eq!(
        hooks.provider_sync_home.lock().unwrap().as_deref(),
        Some(hooks.codex_home.path())
    );
}

#[tokio::test]
async fn launch_applies_saved_active_protocol_proxy_profile_without_live_reconcile() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let inactive = RelayProfile {
        id: "inactive".to_string(),
        name: "Inactive".to_string(),
        protocol: RelayProtocol::ChatCompletions,
        relay_mode: codex_plus_core::settings::RelayMode::PureApi,
        upstream_base_url: "https://inactive.example/v1".to_string(),
        config_contents: r#"model = "inactive-model"
model_provider = "inactive"

[model_providers.inactive]
name = "inactive"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:57321/v1"
"#
        .to_string(),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-inactive"}"#.to_string(),
        ..RelayProfile::default()
    };
    let settings = BackendSettings {
        enhancements_enabled: false,
        relay_profiles_enabled: true,
        relay_common_config_contents: "model_reasoning_effort = \"high\"\n".to_string(),
        relay_context_config_contents: r#"[mcp_servers.context7]
command = "npx"
"#
        .to_string(),
        relay_profiles: vec![
            RelayProfile {
                id: "active".to_string(),
                name: "Active".to_string(),
                protocol: RelayProtocol::ChatCompletions,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                upstream_base_url: "https://claude.example/v1".to_string(),
                config_contents: r#"model = "claude-sonnet-4"
model_provider = "claude"

[model_providers.claude]
name = "claude"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:57321/v1"
"#
                .to_string(),
                auth_contents: r#"{"OPENAI_API_KEY":"sk-claude-saved"}"#.to_string(),
                ..RelayProfile::default()
            },
            inactive,
        ],
        active_relay_id: "active".to_string(),
        ..BackendSettings::default()
    };
    store.save(&settings).unwrap();
    let mut raw_settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("settings.json")).unwrap())
            .unwrap();
    raw_settings["customField"] = serde_json::json!({ "preserved": true });
    raw_settings["relayProfiles"][0]["activeCustomField"] =
        serde_json::json!({ "preserved": "active" });
    raw_settings["relayProfiles"][1]["inactiveCustomField"] =
        serde_json::json!({ "preserved": "inactive" });
    std::fs::write(
        temp.path().join("settings.json"),
        serde_json::to_vec_pretty(&raw_settings).unwrap(),
    )
    .unwrap();
    let settings = store.load().unwrap();
    let inactive_before = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "inactive")
        .unwrap()
        .clone();
    let hooks = FakeHooks::new(events.clone())
        .with_settings(settings)
        .with_live_relay_apply(store.clone());
    std::fs::write(
        hooks.codex_home.path().join("config.toml"),
        r#"model = "gpt-5.6"
model_provider = "openai"
model_reasoning_effort = "high"

[model_providers.openai]
name = "openai"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.openai.example/v1"

[mcp_servers.context7]
command = "npx"
"#,
    )
    .unwrap();
    std::fs::write(
        hooks.codex_home.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-gpt-live"}"#,
    )
    .unwrap();

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 58000,
            status_store: StatusStore::new(temp.path().join("latest-status.json")),
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let live_config = std::fs::read_to_string(hooks.codex_home.path().join("config.toml")).unwrap();
    let live_auth = std::fs::read_to_string(hooks.codex_home.path().join("auth.json")).unwrap();
    assert!(live_config.contains(r#"model = "claude-sonnet-4""#));
    assert!(live_config.contains(r#"model_provider = "claude""#));
    assert!(!live_config.contains(r#"model = "gpt-5.6""#));
    assert!(live_config.contains(r#"model_reasoning_effort = "high""#));
    assert!(live_config.contains("[mcp_servers.context7]"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&live_auth).unwrap()["OPENAI_API_KEY"],
        "sk-claude-saved"
    );
    let stored = store.load().unwrap();
    let active = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "active")
        .unwrap();
    assert!(active.config_contents.contains("claude-sonnet-4"));
    assert!(!active.config_contents.contains("gpt-5.6"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&active.auth_contents).unwrap()["OPENAI_API_KEY"],
        "sk-claude-saved"
    );
    assert_eq!(
        stored
            .relay_profiles
            .iter()
            .find(|profile| profile.id == "inactive")
            .unwrap(),
        &inactive_before
    );
    let raw_stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        raw_stored["customField"],
        serde_json::json!({ "preserved": true })
    );
    assert_eq!(
        raw_stored["relayProfiles"][0]["activeCustomField"],
        serde_json::json!({ "preserved": "active" })
    );
    assert_eq!(
        raw_stored["relayProfiles"][1]["inactiveCustomField"],
        serde_json::json!({ "preserved": "inactive" })
    );
    let events = events.lock().unwrap();
    assert!(events.contains(&"apply-relay".to_string()));
    assert!(events.contains(&"start-helper:57321".to_string()));
}

#[tokio::test]
async fn launch_applies_saved_active_responses_profile_over_stale_live_config() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let settings = BackendSettings {
        enhancements_enabled: false,
        relay_profiles_enabled: true,
        relay_profiles: vec![RelayProfile {
            id: "active".to_string(),
            name: "Active Responses".to_string(),
            model: "claude-sonnet-4".to_string(),
            protocol: RelayProtocol::Responses,
            relay_mode: codex_plus_core::settings::RelayMode::PureApi,
            config_contents: r#"model = "claude-sonnet-4"
model_provider = "claude"

[model_providers.claude]
name = "claude"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://claude.example/v1"
"#
            .to_string(),
            auth_contents: r#"{"OPENAI_API_KEY":"sk-claude-saved"}"#.to_string(),
            ..RelayProfile::default()
        }],
        active_relay_id: "active".to_string(),
        ..BackendSettings::default()
    };
    store.save(&settings).unwrap();
    let hooks = FakeHooks::new(events.clone())
        .with_settings(store.load().unwrap())
        .with_live_relay_apply(store);
    std::fs::write(
        hooks.codex_home.path().join("config.toml"),
        r#"model = "gpt-5.6"
model_provider = "openai"

[model_providers.openai]
name = "openai"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.openai.example/v1"
"#,
    )
    .unwrap();
    std::fs::write(
        hooks.codex_home.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-gpt-live"}"#,
    )
    .unwrap();

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 58000,
            status_store: StatusStore::new(temp.path().join("latest-status.json")),
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let live_config = std::fs::read_to_string(hooks.codex_home.path().join("config.toml")).unwrap();
    let live_auth = std::fs::read_to_string(hooks.codex_home.path().join("auth.json")).unwrap();
    assert!(live_config.contains(r#"model = "claude-sonnet-4""#));
    assert!(live_config.contains(r#"model_provider = "claude""#));
    assert!(!live_config.contains(r#"model = "gpt-5.6""#));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&live_auth).unwrap()["OPENAI_API_KEY"],
        "sk-claude-saved"
    );
    let events = events.lock().unwrap();
    assert!(events.contains(&"apply-relay".to_string()));
    assert!(
        !events
            .iter()
            .any(|event| event.starts_with("start-helper:"))
    );
}

#[tokio::test]
async fn launch_lifecycle_applies_aggregate_relay_before_provider_sync_and_launch() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        provider_sync_enabled: true,
        relay_profiles: vec![
            RelayProfile {
                id: "relay-a".to_string(),
                name: "Relay A".to_string(),
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                ..RelayProfile::default()
            },
            RelayProfile {
                id: "aggregate".to_string(),
                name: "Aggregate".to_string(),
                relay_mode: codex_plus_core::settings::RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        active_relay_id: "aggregate".to_string(),
        aggregate_relay_profiles: vec![codex_plus_core::settings::AggregateRelayProfile {
            id: "aggregate".to_string(),
            name: "Aggregate".to_string(),
            strategy: codex_plus_core::settings::AggregateRelayStrategy::Failover,
            members: vec![codex_plus_core::settings::AggregateRelayMember {
                relay_id: "relay-a".to_string(),
                weight: 1,
            }],
        }],
        active_aggregate_relay_id: "aggregate".to_string(),
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    let apply_index = events
        .iter()
        .position(|event| event == "apply-relay")
        .unwrap();
    let sync_index = events
        .iter()
        .position(|event| event == "provider-sync")
        .unwrap();
    let launch_index = events
        .iter()
        .position(|event| event == "launch:9229")
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "apply-relay")
            .count(),
        1
    );
    assert!(apply_index < sync_index);
    assert!(sync_index < launch_index);
}

#[tokio::test]
async fn launch_lifecycle_skips_protocol_proxy_relay_when_profiles_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_settings(BackendSettings {
        enhancements_enabled: false,
        relay_profiles_enabled: false,
        relay_profiles: vec![RelayProfile {
            id: "relay-chat".to_string(),
            name: "Chat Completions".to_string(),
            protocol: RelayProtocol::ChatCompletions,
            relay_mode: codex_plus_core::settings::RelayMode::MixedApi,
            ..RelayProfile::default()
        }],
        active_relay_id: "relay-chat".to_string(),
        ..BackendSettings::default()
    });

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(!events.contains(&"apply-relay".to_string()));
    assert!(!events.contains(&"computer-use-guard".to_string()));
    assert!(
        !events
            .iter()
            .any(|event| event.starts_with("start-helper:"))
    );
    assert!(events.contains(&"launch:9229".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_applies_relay_with_duplicate_context_parent_tables() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            relay_common_config_contents: "[mcp_servers]\n".to_string(),
            relay_context_config_contents:
                "[mcp_servers]\n\n[mcp_servers.ida]\ncommand = \"python\"\n".to_string(),
            relay_profiles: vec![RelayProfile {
                id: "relay-a".to_string(),
                name: "Relay A".to_string(),
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                config_contents: r#"model = "gpt-5.5"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example/v1"
experimental_bearer_token = "sk-test"
"#
                .to_string(),
                auth_contents: r#"{"OPENAI_API_KEY":"sk-test"}"#.to_string(),
                ..RelayProfile::default()
            }],
            active_relay_id: "relay-a".to_string(),
            ..BackendSettings::default()
        })
        .with_live_relay_apply(SettingsStore::new(temp.path().join("settings.json")));

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    let events = events.lock().unwrap().clone();
    assert!(events.contains(&"apply-relay".to_string()));
    assert!(!events.contains(&"computer-use-guard".to_string()));
    assert!(events.contains(&"launch:9229".to_string()));
    let live_config = std::fs::read_to_string(hooks.codex_home.path().join("config.toml")).unwrap();
    assert_eq!(live_config.matches("[mcp_servers]").count(), 1);
    assert!(live_config.contains("[mcp_servers.ida]"));
}

#[tokio::test]
async fn launch_lifecycle_enters_degraded_mode_and_retries_when_injection_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_inject_error("inject failed");

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: status_store.clone(),
        },
        &hooks,
    )
    .await
    .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "status:running_degraded",
        ]
    );
    let status = status_store.load_latest().unwrap().unwrap();
    assert_eq!(status.status, "running_degraded");
    assert!(status.message.contains("Codex launched"));

    handle.wait_for_codex_exit().await.unwrap();
    let events = events.lock().unwrap().clone();
    assert!(events.contains(&"wait-codex".to_string()));
    assert!(events.contains(&"shutdown-helper:57321".to_string()));
    assert!(!events.contains(&"terminate-codex".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_cleans_helper_when_launch_fails_after_helper_started() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone()).with_launch_error("launch failed");

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: status_store.clone(),
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("launch failed"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "shutdown-helper:57321",
            "status:failed",
        ]
    );
}

#[tokio::test]
async fn launch_starts_helper_when_chat_protocol_proxy_is_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let settings = BackendSettings {
        enhancements_enabled: false,
        relay_profiles: vec![RelayProfile {
            id: "relay-chat".to_string(),
            name: "Chat".to_string(),
            model: String::new(),
            base_url: "https://chat-only.example.test/v1".to_string(),
            upstream_base_url: "https://chat-only.example.test/v1".to_string(),
            api_key: "sk-test".to_string(),
            protocol: RelayProtocol::ChatCompletions,
            relay_mode: codex_plus_core::settings::RelayMode::MixedApi,
            official_mix_api_key: false,
            test_model: String::new(),
            config_contents: String::new(),
            auth_contents: String::new(),
            use_common_config: true,
            context_selection: codex_plus_core::settings::RelayContextSelection::default(),
            context_selection_initialized: false,
            context_window: String::new(),
            auto_compact_limit: String::new(),
            model_insert_mode: codex_plus_core::settings::RelayModelInsertMode::default(),
            model_list: String::new(),
            model_windows: String::new(),
            model_vlm: String::new(),
            vlm_api_key: String::new(),
            vlm_model: String::new(),
            vlm_base_url: String::new(),
            user_agent: String::new(),
        }],
        active_relay_id: "relay-chat".to_string(),
        ..BackendSettings::default()
    };
    let hooks = FakeHooks::new(events.clone()).with_settings(settings);

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 58000,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();

    let before_stop = events.lock().unwrap().clone();
    assert!(before_stop.contains(&"select-helper:58000".to_string()));
    assert!(before_stop.contains(&"start-helper:57321".to_string()));
    assert!(!before_stop.contains(&"inject:9229:57321".to_string()));

    handle.wait_for_codex_exit().await.unwrap();

    let after_stop = events.lock().unwrap().clone();
    assert!(after_stop.contains(&"wait-codex".to_string()));
    assert!(after_stop.contains(&"shutdown-helper:57321".to_string()));
}

#[tokio::test]
async fn launch_lifecycle_cleans_helper_and_codex_when_status_save_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(temp.path().join("status-parent-file"), "not a directory").unwrap();
    let status_store = StatusStore::new(
        temp.path()
            .join("status-parent-file")
            .join("latest-status.json"),
    );
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks =
        FakeHooks::new(events.clone()).with_launch_result(CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229".to_string(),
            process_id: Some(4242),
        });

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("failed to create directory"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "shutdown-helper:57321",
            "terminate-packaged:4242",
            "status:failed",
        ]
    );
}

#[tokio::test]
async fn launch_lifecycle_keeps_packaged_process_id_running_and_retries_when_injection_fails() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("latest-status.json"));
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_launch_result(CodexLaunch::PackagedActivation {
            app_user_model_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            arguments: "--remote-debugging-port=9229".to_string(),
            process_id: Some(4242),
        })
        .with_inject_error("inject failed");

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store,
        },
        &hooks,
    )
    .await
    .unwrap();

    assert!(
        !events
            .lock()
            .unwrap()
            .contains(&"terminate-packaged:4242".to_string())
    );
    handle.wait_for_codex_exit().await.unwrap();
}

#[tokio::test]
async fn default_provider_sync_enabled_fails_instead_of_silently_skipping() {
    let hooks = FakeHooks::new(Arc::new(Mutex::new(Vec::new()))).with_provider_sync_unsupported();

    let error = hooks
        .run_provider_sync(Path::new("."))
        .await
        .expect_err("default-style provider sync should be explicit");

    assert!(
        error
            .to_string()
            .contains("provider sync requires launcher hooks")
    );
}

#[tokio::test]
async fn launch_continues_when_plugin_marketplace_config_fails() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_plugin_marketplace_error("config.toml TOML parse failed");

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(PathBuf::from("/Applications/Codex.app")),
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::new(tempfile::tempdir().unwrap().path().join("status.json")),
        },
        &hooks,
    )
    .await
    .unwrap();

    assert_eq!(handle.debug_port, 9229);
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "select-debug:9229",
            "select-helper:57321",
            "load-settings",
            "plugin-marketplace",
            "start-helper:57321",
            "launch:9229",
            "inject:9229:57321",
            "status:running"
        ]
    );
}

#[test]
fn launcher_macos_cleanup_command_targets_specific_app_bundle() {
    let command = build_macos_cleanup_command(
        Path::new("/Applications/OpenAI Codex.app"),
        MacosCleanupPolicy::QuitIfNotPreviouslyRunning,
    )
    .expect("cleanup command should be allowed");

    assert_eq!(command[0], "osascript");
    assert!(command.iter().any(|part| part.contains("OpenAI Codex")));
    assert!(!command.iter().any(|part| part == "Codex"));
}

#[test]
fn launcher_macos_cleanup_is_skipped_when_app_was_already_running() {
    let command = build_macos_cleanup_command(
        Path::new("/Applications/OpenAI Codex.app"),
        MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning,
    );

    assert_eq!(command, None);
}

#[tokio::test]
async fn launcher_macos_exit_confirmation_retries_unknown_and_running_states() {
    let observations = Arc::new(Mutex::new(std::collections::VecDeque::from([
        Err("app state unavailable"),
        Ok(true),
        Ok(false),
    ])));
    let observed = Arc::clone(&observations);

    codex_plus_core::launcher::wait_for_confirmed_macos_app_exit_with(
        move || {
            let observed = Arc::clone(&observed);
            async move {
                observed
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("test should provide an observation")
            }
        },
        std::time::Duration::ZERO,
    )
    .await;

    assert!(observations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn launcher_macos_cleanup_keeps_waiting_when_bounded_exit_checks_fail() {
    let observations = Arc::new(Mutex::new(std::collections::VecDeque::from([
        Err("app state unavailable"),
        Ok(true),
        Ok(false),
    ])));
    let observed = Arc::clone(&observations);

    let waiter_error = codex_plus_core::launcher::wait_for_macos_exit_after_bounded_failures_with(
        Some(async { anyhow::bail!("bundle waiter failed") }),
        move || {
            let observed = Arc::clone(&observed);
            async move {
                observed
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("test should provide an observation")
            }
        },
        std::time::Duration::ZERO,
    )
    .await
    .expect("failed bundle waiter should be reported after app exit confirmation");

    assert!(waiter_error.to_string().contains("bundle waiter failed"));
    assert!(observations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn default_launch_hooks_provider_sync_enabled_returns_explicit_error() {
    let error = DefaultLaunchHooks::default()
        .run_provider_sync(Path::new("."))
        .await
        .expect_err("default provider sync should not silently skip");

    assert!(
        error
            .to_string()
            .contains("provider sync requires launcher hooks")
    );
}

#[test]
fn paused_dream_skin_does_not_reapply_the_native_base_theme_on_launch() {
    let source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/launcher.rs")).unwrap();

    assert!(source.contains("!settings.codex_app_dream_skin_paused"));
}

#[tokio::test]
async fn launch_lifecycle_routes_host_maintenance_through_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let hooks = FakeHooks::new(Arc::new(Mutex::new(Vec::new())));

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::new(temp.path().join("status.json")),
        },
        &hooks,
    )
    .await
    .unwrap();
    handle.wait_for_codex_exit().await.unwrap();

    assert_eq!(
        hooks.maintenance_events.lock().unwrap().as_slice(),
        [
            "dream-skin",
            "sqlite-model-suffixes",
            "local-storage-model-suffixes"
        ]
    );
}

#[test]
fn launch_holds_relay_switch_lock_until_non_enhanced_codex_is_ready() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (hooks, startup_entered, release_startup) = FakeHooks::new(events)
        .with_settings(BackendSettings {
            enhancements_enabled: false,
            ..BackendSettings::default()
        })
        .with_startup_barrier();
    let contender_home = hooks.codex_home.path().to_path_buf();
    let launcher_hooks = hooks.clone();
    let launcher = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let handle = launch_and_inject_with_hooks(
                LaunchOptions {
                    app_dir: Some(app_dir),
                    debug_port: 9229,
                    helper_port: 57321,
                    status_store: StatusStore::new(temp.path().join("status.json")),
                },
                &launcher_hooks,
            )
            .await?;
            handle.wait_for_codex_exit().await
        })
    });

    startup_entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let contender = std::thread::spawn(move || {
        let _guard = acquire_relay_switch_lock(&contender_home).unwrap();
        acquired_tx.send(()).unwrap();
    });
    assert!(
        acquired_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    release_startup.send(()).unwrap();
    launcher.join().unwrap().unwrap();
    acquired_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    contender.join().unwrap();
}

#[test]
fn readiness_failure_stops_codex_and_protocol_proxy_before_releasing_relay_lock() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("status.json"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let (hooks, termination_entered, release_termination) = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            enhancements_enabled: false,
            relay_profiles_enabled: true,
            relay_profiles: vec![RelayProfile {
                id: "chat".to_string(),
                protocol: RelayProtocol::ChatCompletions,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                ..RelayProfile::default()
            }],
            active_relay_id: "chat".to_string(),
            ..BackendSettings::default()
        })
        .with_startup_error("page not ready")
        .with_termination_barrier();
    let contender_home = hooks.codex_home.path().to_path_buf();
    let launcher_hooks = hooks.clone();
    let launcher_status = status_store.clone();
    let launcher = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(launch_and_inject_with_hooks(
            LaunchOptions {
                app_dir: Some(app_dir),
                debug_port: 9229,
                helper_port: 58000,
                status_store: launcher_status,
            },
            &launcher_hooks,
        ))
    });

    termination_entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("launcher must enter Codex termination after readiness failure");
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let (attempting_tx, attempting_rx) = std::sync::mpsc::channel();
    let contender = std::thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        let _guard = acquire_relay_switch_lock(&contender_home).unwrap();
        acquired_tx.send(()).unwrap();
    });
    attempting_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(
        acquired_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err(),
        "relay lock must remain held while the failed Codex process is being terminated"
    );
    release_termination.send(()).unwrap();
    let error = launcher
        .join()
        .unwrap()
        .expect_err("unconfirmed config consumption must fail the launch");
    assert!(error.to_string().contains("page not ready"));
    acquired_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    contender.join().unwrap();

    assert_eq!(
        status_store.load_latest().unwrap().unwrap().status,
        "failed"
    );
    let events = events.lock().unwrap().clone();
    assert!(events.contains(&"start-helper:57321".to_string()));
    assert!(events.contains(&"shutdown-helper:57321".to_string()));
    assert!(events.contains(&"terminate-codex".to_string()));
}

#[test]
fn status_save_failure_stops_codex_after_page_ready_without_blocking_relay_switches() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let failing_status_path = temp.path().join("status-is-a-directory");
    std::fs::create_dir(&failing_status_path).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (hooks, termination_entered, release_termination) = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            enhancements_enabled: false,
            relay_profiles_enabled: true,
            relay_profiles: vec![RelayProfile {
                id: "chat".to_string(),
                protocol: RelayProtocol::ChatCompletions,
                relay_mode: codex_plus_core::settings::RelayMode::PureApi,
                ..RelayProfile::default()
            }],
            active_relay_id: "chat".to_string(),
            ..BackendSettings::default()
        })
        .with_termination_barrier();
    let contender_home = hooks.codex_home.path().to_path_buf();
    let launcher_hooks = hooks.clone();
    let launcher = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(launch_and_inject_with_hooks(
            LaunchOptions {
                app_dir: Some(app_dir),
                debug_port: 9229,
                helper_port: 58000,
                status_store: StatusStore::new(failing_status_path),
            },
            &launcher_hooks,
        ))
    });

    termination_entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("launcher must terminate Codex after status persistence fails");
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let (attempting_tx, attempting_rx) = std::sync::mpsc::channel();
    let contender = std::thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        let _guard = acquire_relay_switch_lock(&contender_home).unwrap();
        acquired_tx.send(()).unwrap();
    });
    attempting_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    let acquired_while_terminating = acquired_rx
        .recv_timeout(std::time::Duration::from_millis(100))
        .is_ok();
    release_termination.send(()).unwrap();
    launcher
        .join()
        .unwrap()
        .expect_err("status persistence failure must fail launch");
    if !acquired_while_terminating {
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
    }
    contender.join().unwrap();

    let events = events.lock().unwrap();
    assert!(events.contains(&"shutdown-helper:57321".to_string()));
    assert!(events.contains(&"terminate-codex".to_string()));
    // 页面已确认加载配置后，切换锁的保护目标已达成并被提前释放；
    // 此后即使状态持久化失败、终止 Codex，也不应再阻塞 Manager 侧的切换。
    assert!(
        acquired_while_terminating,
        "relay lock must be free while a post-ready Codex failure is being cleaned up"
    );
}

#[tokio::test]
async fn launch_failure_reports_codex_termination_failure() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let hooks = FakeHooks::new(Arc::new(Mutex::new(Vec::new())))
        .with_settings(BackendSettings {
            enhancements_enabled: false,
            ..BackendSettings::default()
        })
        .with_startup_error("page not ready")
        .with_termination_error("access denied");

    let error = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::new(temp.path().join("status.json")),
        },
        &hooks,
    )
    .await
    .expect_err("launch and termination failures must be reported");
    let message = error.to_string();

    assert!(message.contains("page not ready"));
    assert!(message.contains("access denied"));
}

#[tokio::test]
async fn delayed_page_readiness_keeps_launch_when_enhancement_injection_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let status_store = StatusStore::new(temp.path().join("status.json"));
    let events = Arc::new(Mutex::new(Vec::new()));
    let hooks = FakeHooks::new(events.clone())
        .with_settings(BackendSettings {
            enhancements_enabled: true,
            ..BackendSettings::default()
        })
        .with_startup_error("initial page wait timed out");

    let handle = launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: status_store.clone(),
        },
        &hooks,
    )
    .await
    .expect("successful injection proves the page consumed its configuration");

    assert_eq!(
        status_store.load_latest().unwrap().unwrap().status,
        "running"
    );
    assert!(
        !events
            .lock()
            .unwrap()
            .contains(&"terminate-codex".to_string())
    );
    handle.wait_for_codex_exit().await.unwrap();
}

struct StartupBarrier {
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

#[derive(Clone)]
struct FakeHooks {
    events: Arc<Mutex<Vec<String>>>,
    maintenance_events: Arc<Mutex<Vec<String>>>,
    codex_home: Arc<tempfile::TempDir>,
    applied_relay_home: Arc<Mutex<Option<PathBuf>>>,
    provider_sync_home: Arc<Mutex<Option<PathBuf>>>,
    live_relay_store: Option<SettingsStore>,
    settings: BackendSettings,
    launch_result: CodexLaunch,
    launch_error: Option<String>,
    inject_error: Option<String>,
    provider_sync_unsupported: bool,
    plugin_marketplace_error: Option<String>,
    startup_barrier: Option<Arc<StartupBarrier>>,
    termination_barrier: Option<Arc<StartupBarrier>>,
    termination_error: Option<String>,
    startup_error: Option<String>,
    probe_relay_lock_at_injection: bool,
}

impl FakeHooks {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            maintenance_events: Arc::new(Mutex::new(Vec::new())),
            codex_home: Arc::new(tempfile::tempdir().unwrap()),
            applied_relay_home: Arc::new(Mutex::new(None)),
            provider_sync_home: Arc::new(Mutex::new(None)),
            live_relay_store: None,
            settings: BackendSettings::default(),
            launch_result: CodexLaunch::Process {
                command: vec!["codex".to_string()],
                wait_strategy: codex_plus_core::launcher::ProcessWaitStrategy::TrackedChild,
                macos_cleanup_policy: None,
            },
            launch_error: None,
            inject_error: None,
            provider_sync_unsupported: false,
            plugin_marketplace_error: None,
            startup_barrier: None,
            termination_barrier: None,
            termination_error: None,
            startup_error: None,
            probe_relay_lock_at_injection: false,
        }
    }

    fn with_relay_lock_probe_at_injection(mut self) -> Self {
        self.probe_relay_lock_at_injection = true;
        self
    }

    fn with_settings(mut self, settings: BackendSettings) -> Self {
        self.settings = settings;
        self
    }

    fn with_launch_result(mut self, launch_result: CodexLaunch) -> Self {
        self.launch_result = launch_result;
        self
    }

    fn with_live_relay_apply(mut self, store: SettingsStore) -> Self {
        self.live_relay_store = Some(store);
        self
    }

    fn with_inject_error(mut self, message: &str) -> Self {
        self.inject_error = Some(message.to_string());
        self
    }

    fn with_launch_error(mut self, message: &str) -> Self {
        self.launch_error = Some(message.to_string());
        self
    }

    fn with_provider_sync_unsupported(mut self) -> Self {
        self.provider_sync_unsupported = true;
        self
    }

    fn with_plugin_marketplace_error(mut self, message: &str) -> Self {
        self.plugin_marketplace_error = Some(message.to_string());
        self
    }

    fn with_startup_barrier(
        mut self,
    ) -> (
        Self,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        self.startup_barrier = Some(Arc::new(StartupBarrier {
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(release_rx),
        }));
        (self, entered_rx, release_tx)
    }

    fn with_startup_error(mut self, message: &str) -> Self {
        self.startup_error = Some(message.to_string());
        self
    }

    fn with_termination_barrier(
        mut self,
    ) -> (
        Self,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        self.termination_barrier = Some(Arc::new(StartupBarrier {
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(release_rx),
        }));
        (self, entered_rx, release_tx)
    }

    fn with_termination_error(mut self, message: &str) -> Self {
        self.termination_error = Some(message.to_string());
        self
    }

    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }

    fn maintenance_event(&self, event: impl Into<String>) {
        self.maintenance_events.lock().unwrap().push(event.into());
    }
}

#[async_trait::async_trait(?Send)]
impl LaunchHooks for FakeHooks {
    fn resolve_codex_home(&self) -> PathBuf {
        self.codex_home.path().to_path_buf()
    }

    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        _settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf> {
        app_dir
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("missing app dir"))
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        self.event(format!("select-debug:{requested}"));
        requested
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        self.event(format!("select-helper:{requested}"));
        requested
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        self.event("load-settings");
        Ok(self.settings.clone())
    }

    async fn run_provider_sync(&self, codex_home: &Path) -> anyhow::Result<()> {
        self.event("provider-sync");
        *self.provider_sync_home.lock().unwrap() = Some(codex_home.to_path_buf());
        if self.provider_sync_unsupported {
            anyhow::bail!("provider sync requires launcher hooks");
        }
        Ok(())
    }

    async fn apply_active_relay_profile(
        &self,
        settings: &BackendSettings,
        codex_home: &Path,
    ) -> anyhow::Result<()> {
        self.event("apply-relay");
        *self.applied_relay_home.lock().unwrap() = Some(codex_home.to_path_buf());
        if self.live_relay_store.is_some() {
            let profile = settings.active_relay_profile();
            let common_config = codex_plus_core::relay_config::normalize_config_text(
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
            codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules(
                codex_home,
                &profile,
                &common_config,
            )?;
        }
        Ok(())
    }

    async fn ensure_computer_use_config(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
        self.event("computer-use-guard");
        Ok(())
    }

    async fn ensure_plugin_marketplace_config(
        &self,
        _settings: &BackendSettings,
        _relay_switch_lock: &codex_plus_core::relay_switch::RelaySwitchLockGuard,
    ) -> anyhow::Result<()> {
        if let Some(message) = &self.plugin_marketplace_error {
            self.event("plugin-marketplace");
            anyhow::bail!(message.clone());
        }
        Ok(())
    }

    fn sync_dream_skin_base_theme(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
        self.maintenance_event("dream-skin");
        Ok(())
    }

    fn sanitize_historical_model_suffixes(
        &self,
        _codex_home: &Path,
    ) -> anyhow::Result<codex_plus_core::codex_sqlite::SanitizeModelSuffixResult> {
        self.maintenance_event("sqlite-model-suffixes");
        Ok(Default::default())
    }

    async fn sanitize_local_storage_model_suffixes(&self, _debug_port: u16) {
        self.maintenance_event("local-storage-model-suffixes");
    }

    async fn wait_for_codex_config_load(&self, _debug_port: u16) -> anyhow::Result<()> {
        if let Some(barrier) = &self.startup_barrier {
            if let Some(entered) = barrier.entered.lock().unwrap().take() {
                entered.send(()).unwrap();
            }
            barrier
                .release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }
        if let Some(message) = &self.startup_error {
            anyhow::bail!(message.clone());
        }
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        self.event(format!("start-helper:{helper_port}"));
        Ok(())
    }

    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        settings: &BackendSettings,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch> {
        assert!(app_dir.ends_with("Codex.app"));
        let launch_detail = if extra_args.is_empty() {
            format!("launch:{debug_port}")
        } else {
            format!("launch:{debug_port}:{}", extra_args.join(","))
        };
        if settings.codex_app_native_menu_localization {
            self.event(launch_detail);
        } else {
            self.event(format!("{launch_detail}:native-menu-off"));
        }
        if let Some(message) = &self.launch_error {
            anyhow::bail!(message.clone());
        }
        Ok(self.launch_result.clone())
    }

    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        self.event(format!("inject:{debug_port}:{helper_port}"));
        if let Some(message) = &self.inject_error {
            anyhow::bail!(message.clone());
        }
        Ok(())
    }

    async fn ensure_injection(&self, debug_port: u16, helper_port: u16, _app_dir: &Path) -> bool {
        if self.probe_relay_lock_at_injection {
            // 页面就绪后进入注入阶段时，跨进程切换锁必须已释放，
            // 否则注入重试期间 Manager 的切换/保存会被一直阻塞。
            match codex_plus_core::relay_switch::acquire_relay_switch_lock_with_timeout(
                &self.resolve_codex_home(),
                std::time::Duration::from_millis(200),
            ) {
                Ok(guard) => {
                    drop(guard);
                    self.event("injection-relay-lock:free");
                }
                Err(_) => self.event("injection-relay-lock:held"),
            }
        }
        self.event(format!("inject:{debug_port}:{helper_port}"));
        self.inject_error.is_none()
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
        self.event("computer-use-guard-watchdog");
        Ok(())
    }

    async fn write_status(&self, status: &str) {
        self.event(format!("status:{status}"));
    }

    async fn wait_for_codex_exit(&self, _launch: &CodexLaunch) -> anyhow::Result<()> {
        self.event("wait-codex");
        Ok(())
    }

    async fn shutdown_helper(&self, helper_port: u16) {
        self.event(format!("shutdown-helper:{helper_port}"));
    }

    async fn terminate_codex(&self, launch: &CodexLaunch) -> anyhow::Result<()> {
        if let Some(barrier) = &self.termination_barrier {
            if let Some(entered) = barrier.entered.lock().unwrap().take() {
                entered.send(()).unwrap();
            }
            barrier
                .release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }
        if let Some(process_id) = launch.process_id() {
            self.event(format!("terminate-packaged:{process_id}"));
        } else {
            self.event("terminate-codex");
        }
        if let Some(message) = &self.termination_error {
            anyhow::bail!(message.clone());
        }
        Ok(())
    }
}

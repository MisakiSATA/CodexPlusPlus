#[cfg(target_os = "linux")]
use codex_plus_core::install::linux::{install_desktop_entries, uninstall_desktop_entries};
use codex_plus_core::install::{
    InstallOptions, MANAGER_BUNDLE_ID, SILENT_BINARY, SILENT_BUNDLE_ID, app_bundle_names,
    build_linux_entrypoint_plan, build_macos_app_bundle, build_windows_entrypoint_plan,
    companion_binary_path_from_exe, default_install_root_strategy, desktop_entry_names,
    macos_companion_bundle_identifier_from_exe, shortcut_names,
};

#[test]
fn windows_entrypoint_plan_contains_silent_and_manager_entrypoints() {
    let options = InstallOptions {
        install_root: Some("C:/Users/A/Desktop".into()),
        launcher_path: Some("C:/Tools/codex-plus-plus.exe".into()),
        manager_path: Some("C:/Tools/codex-plus-plus-manager.exe".into()),
        remove_owned_data: false,
    };

    let plan = build_windows_entrypoint_plan(&options);

    assert!(plan.silent_shortcut.ends_with("Codex++.lnk"));
    assert!(plan.manager_shortcut.ends_with("Codex++ 管理工具.lnk"));
    assert_eq!(plan.launcher_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(plan.manager_path, "C:/Tools/codex-plus-plus-manager.exe");
    assert_eq!(plan.silent_icon_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(
        plan.manager_icon_path,
        "C:/Tools/codex-plus-plus-manager.exe"
    );
    assert_eq!(plan.uninstall_key, "CodexPlusPlus");
    assert_eq!(plan.legacy_uninstall_key, "Codex++");
    assert_eq!(
        plan.uninstaller_path.replace('\\', "/"),
        "C:/Tools/uninstall.exe"
    );
    assert_eq!(
        plan.uninstall_command.replace('\\', "/"),
        "\"C:/Tools/uninstall.exe\""
    );
    assert_eq!(
        plan.quiet_uninstall_command.replace('\\', "/"),
        "\"C:/Tools/uninstall.exe\" /S"
    );
    assert_ne!(
        plan.uninstall_command,
        "\"C:/Tools/codex-plus-plus-manager.exe\""
    );
}

#[test]
fn windows_entrypoint_plan_can_request_owned_data_removal_without_shell_script() {
    let options = InstallOptions {
        install_root: Some("C:/Users/A/Desktop".into()),
        launcher_path: None,
        manager_path: None,
        remove_owned_data: true,
    };

    let plan = build_windows_entrypoint_plan(&options);

    assert!(plan.silent_shortcut.ends_with("Codex++.lnk"));
    assert!(plan.manager_shortcut.ends_with("Codex++ 管理工具.lnk"));
    assert!(plan.remove_owned_data);
}

#[test]
fn macos_bundle_metadata_contains_silent_and_manager_apps() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/opt/Codex++/codex-plus-plus".into()),
        manager_path: Some("/opt/Codex++/codex-plus-plus-manager".into()),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert!(silent.app_path.ends_with("Codex++.app"));
    assert!(manager.app_path.ends_with("Codex++ 管理工具.app"));
    assert!(silent.info_plist.contains("<string>Codex++</string>"));
    assert!(
        manager
            .info_plist
            .contains("<string>Codex++ 管理工具</string>")
    );
    assert_eq!(
        silent.binary_target_name.as_deref(),
        Some("codex-plus-plus")
    );
    assert_eq!(
        manager.binary_target_name.as_deref(),
        Some("codex-plus-plus-manager")
    );
    assert!(silent.launch_script.contains("$DIR/codex-plus-plus"));
    assert!(
        manager
            .launch_script
            .contains("$DIR/codex-plus-plus-manager")
    );
}

#[test]
fn installer_exports_expected_two_entrypoint_names() {
    assert_eq!(shortcut_names(), ("Codex++.lnk", "Codex++ 管理工具.lnk"));
    assert_eq!(app_bundle_names(), ("Codex++.app", "Codex++ 管理工具.app"));
    assert_eq!(
        desktop_entry_names(),
        ("codex-plus-plus.desktop", "codex-plus-plus-manager.desktop")
    );
}

#[test]
fn linux_entrypoint_plan_contains_launcher_and_manager_apps() {
    let options = InstallOptions {
        install_root: Some("/home/alice/.local/share/applications".into()),
        launcher_path: Some("/home/alice/Codex Plus/codex-plus-plus".into()),
        manager_path: Some("/home/alice/Codex Plus/codex-plus-plus-manager".into()),
        remove_owned_data: false,
    };

    let plan = build_linux_entrypoint_plan(&options);

    assert!(plan.silent.path.ends_with("codex-plus-plus.desktop"));
    assert!(
        plan.manager
            .path
            .ends_with("codex-plus-plus-manager.desktop")
    );
    assert!(
        plan.silent
            .contents
            .contains("Exec=\"/home/alice/Codex Plus/codex-plus-plus\" %U")
    );
    assert!(
        plan.manager
            .contents
            .contains("Exec=\"/home/alice/Codex Plus/codex-plus-plus-manager\"")
    );
    assert!(
        plan.silent
            .contents
            .contains("MimeType=x-scheme-handler/codexplusplus;")
    );
    assert!(!plan.manager.contents.contains("MimeType="));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_entrypoints_install_and_uninstall_in_xdg_applications_dir() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let applications = temp.path().join("share/applications");
    std::fs::create_dir_all(&bin).unwrap();
    let launcher = bin.join("codex-plus-plus");
    let manager = bin.join("codex-plus-plus-manager");
    std::fs::write(&launcher, "").unwrap();
    std::fs::write(&manager, "").unwrap();
    let options = InstallOptions {
        install_root: Some(applications.clone()),
        launcher_path: Some(launcher),
        manager_path: Some(manager),
        remove_owned_data: false,
    };

    install_desktop_entries(&options).unwrap();
    assert!(applications.join("codex-plus-plus.desktop").is_file());
    assert!(
        applications
            .join("codex-plus-plus-manager.desktop")
            .is_file()
    );

    uninstall_desktop_entries(&options).unwrap();
    assert!(!applications.join("codex-plus-plus.desktop").exists());
    assert!(
        !applications
            .join("codex-plus-plus-manager.desktop")
            .exists()
    );
}

#[test]
fn companion_binary_path_resolves_macos_silent_app_next_to_manager_app() {
    let manager_exe = std::path::Path::new(
        "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );

    let companion = companion_binary_path_from_exe(manager_exe, SILENT_BINARY);

    assert_eq!(
        companion,
        std::path::PathBuf::from("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus")
    );
    assert_ne!(
        companion,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/codex-plus-plus"
        )
    );
}

#[test]
fn companion_binary_path_resolves_macos_manager_app_next_to_silent_app() {
    let silent_exe = std::path::Path::new("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus");

    let companion =
        companion_binary_path_from_exe(silent_exe, codex_plus_core::install::MANAGER_BINARY);

    assert_eq!(
        companion,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        )
    );
}

#[test]
fn macos_companion_launch_uses_bundle_ids_from_app_translocation() {
    let manager_exe = std::path::Path::new(
        "/private/var/folders/x/AppTranslocation/manager-id/d/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );
    let silent_exe = std::path::Path::new(
        "/private/var/folders/x/AppTranslocation/silent-id/d/Codex++.app/Contents/MacOS/CodexPlusPlus",
    );

    assert_eq!(
        macos_companion_bundle_identifier_from_exe(manager_exe, SILENT_BINARY),
        Some(SILENT_BUNDLE_ID)
    );
    assert_eq!(
        macos_companion_bundle_identifier_from_exe(
            silent_exe,
            codex_plus_core::install::MANAGER_BINARY,
        ),
        Some(MANAGER_BUNDLE_ID)
    );
}

#[test]
fn macos_companion_launch_keeps_bare_binary_development_mode() {
    let manager_exe = std::path::Path::new("/tmp/target/debug/codex-plus-plus-manager");

    assert_eq!(
        macos_companion_bundle_identifier_from_exe(manager_exe, SILENT_BINARY),
        None
    );
}

#[test]
fn macos_bundle_does_not_wrap_the_bundle_executable_in_itself() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus".into()),
        manager_path: Some(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager".into(),
        ),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert_eq!(
        silent.binary_source,
        Some(std::path::PathBuf::from(
            "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus"
        ))
    );
    assert_eq!(
        manager.binary_source,
        Some(std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        ))
    );
    assert!(silent.launch_script.contains("$DIR/codex-plus-plus"));
    assert!(
        manager
            .launch_script
            .contains("$DIR/codex-plus-plus-manager")
    );
}

#[test]
fn windows_default_install_root_uses_known_folder_before_userprofile_desktop() {
    let strategy = default_install_root_strategy();

    if cfg!(windows) {
        assert_eq!(strategy, "windows-known-folder");
    } else if cfg!(target_os = "macos") {
        assert_eq!(strategy, "macos-applications");
    } else if cfg!(target_os = "linux") {
        assert_eq!(strategy, "linux-xdg-applications");
    } else {
        assert_eq!(strategy, "user-dirs-desktop");
    }
}

#[test]
fn linux_watcher_autostart_entry_targets_launcher_with_debug_port() {
    use codex_plus_core::install::linux;

    let entry = linux::build_watcher_autostart_entry(
        std::path::Path::new("/home/alice/.local/lib/codex-plus-plus/current/bin/codex-plus-plus"),
        9223,
    );
    assert!(entry.contains(
        "Exec=\"/home/alice/.local/lib/codex-plus-plus/current/bin/codex-plus-plus\" --debug-port 9223"
    ));
    assert!(entry.contains("NoDisplay=true"));
    assert_eq!(
        linux::watcher_autostart_path(std::path::Path::new("/home/alice/.config")),
        std::path::PathBuf::from("/home/alice/.config/autostart/codex-plus-plus-watcher.desktop")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_user_scoped_install_activates_version_and_points_entries_at_current() {
    use codex_plus_core::install::linux;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    let launcher = source.join("codex-plus-plus");
    let manager = source.join("codex-plus-plus-manager");
    std::fs::write(&launcher, "launcher-v1").unwrap();
    std::fs::write(&manager, "manager-v1").unwrap();
    let roots = linux::LinuxInstallRoots {
        applications_dir: temp.path().join("share/applications"),
        icons_dir: temp.path().join("share/icons"),
        library_root: temp.path().join("lib/codex-plus-plus"),
    };
    let options = InstallOptions {
        install_root: None,
        launcher_path: Some(launcher.clone()),
        manager_path: Some(manager.clone()),
        remove_owned_data: false,
    };

    let installed = linux::install_user_scoped(&options, &roots, "1.2.42").unwrap();

    // current 必须是指向版本目录的相对符号链接，入口通过它引用稳定路径。
    let current = roots.library_root.join("current");
    assert_eq!(
        std::fs::read_link(&current).unwrap(),
        std::path::PathBuf::from("versions/1.2.42")
    );
    assert_eq!(
        std::fs::read_to_string(current.join("bin/codex-plus-plus")).unwrap(),
        "launcher-v1"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(current.join("bin/codex-plus-plus"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "installed binary must be executable");

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(installed.version_dir.join("install-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["managedBy"], "Codex++ user install");
    assert_eq!(manifest["version"], "1.2.42");

    let silent_entry =
        std::fs::read_to_string(roots.applications_dir.join("codex-plus-plus.desktop")).unwrap();
    assert!(silent_entry.contains(&format!(
        "Exec=\"{}\" %U",
        current.join("bin/codex-plus-plus").to_string_lossy()
    )));
    assert!(installed.icon_path.is_file());

    // 重复安装（升级重放）必须幂等且保持 current 有效。
    std::fs::write(&launcher, "launcher-v2").unwrap();
    linux::install_user_scoped(&options, &roots, "1.2.42").unwrap();
    assert_eq!(
        std::fs::read_to_string(current.join("bin/codex-plus-plus")).unwrap(),
        "launcher-v2"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_manifest_uninstall_preserves_unmanaged_files() {
    use codex_plus_core::install::linux;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    let launcher = source.join("codex-plus-plus");
    let manager = source.join("codex-plus-plus-manager");
    std::fs::write(&launcher, "").unwrap();
    std::fs::write(&manager, "").unwrap();
    let roots = linux::LinuxInstallRoots {
        applications_dir: temp.path().join("share/applications"),
        icons_dir: temp.path().join("share/icons"),
        library_root: temp.path().join("lib/codex-plus-plus"),
    };
    let options = InstallOptions {
        install_root: None,
        launcher_path: Some(launcher),
        manager_path: Some(manager),
        remove_owned_data: false,
    };
    let installed = linux::install_user_scoped(&options, &roots, "1.2.42").unwrap();

    // 用户自己放进版本目录的文件不属于 manifest，卸载时必须保留。
    let foreign = installed.version_dir.join("bin/user-note.txt");
    std::fs::write(&foreign, "keep me").unwrap();
    // 非 Codex++ 管理的版本目录整体保留。
    let unmanaged_dir = roots.library_root.join("versions/manual");
    std::fs::create_dir_all(&unmanaged_dir).unwrap();
    std::fs::write(unmanaged_dir.join("data"), "").unwrap();

    let config_home = temp.path().join("config");
    let autostart = linux::watcher_autostart_path(&config_home);
    std::fs::create_dir_all(autostart.parent().unwrap()).unwrap();
    std::fs::write(&autostart, "").unwrap();

    linux::uninstall_user_scoped(&options, &roots, Some(&config_home)).unwrap();
    assert!(!installed.silent_entry.exists());
    assert!(!installed.manager_entry.exists());
    assert!(!installed.icon_path.exists());
    assert!(!autostart.exists());

    linux::uninstall_user_application(&roots.library_root).unwrap();
    assert!(!roots.library_root.join("current").symlink_metadata().is_ok());
    assert!(!installed.version_dir.join("bin/codex-plus-plus").exists());
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "keep me");
    assert!(unmanaged_dir.join("data").exists());
}

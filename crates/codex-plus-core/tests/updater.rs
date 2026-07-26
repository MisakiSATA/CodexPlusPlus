use codex_plus_core::update::{
    Release, download_asset_to, is_newer_version, parse_version_tag, release_from_github_payload,
    release_from_latest_json_payload, safe_asset_name, select_update_asset,
};
use serde_json::json;

#[test]
fn parse_version_tag_accepts_prefix_and_suffix() {
    assert_eq!(parse_version_tag("v1.2.3").unwrap(), vec![1, 2, 3]);
    assert_eq!(parse_version_tag("1.2.3").unwrap(), vec![1, 2, 3]);
    assert_eq!(parse_version_tag("v1.2.3-beta.1").unwrap(), vec![1, 2, 3]);
}

#[test]
fn version_comparison_uses_numeric_segments() {
    assert!(is_newer_version("v1.0.10", "1.0.4").unwrap());
    assert!(!is_newer_version("v1.0.4", "1.0.4").unwrap());
    assert!(!is_newer_version("v1.0.3", "1.0.4").unwrap());
}

#[test]
fn github_payload_selects_platform_installer() {
    let release = release_from_github_payload(&json!({
        "tag_name": "v1.0.9",
        "html_url": "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.0.9",
        "body": "fixes",
        "assets": [
            {"name": "source.zip", "browser_download_url": "https://example.test/source.zip"},
            {"name": "codex-plus-plus-manager.exe", "browser_download_url": "https://example.test/manager.exe"},
            {"name": "CodexPlusPlus_1.0.9_x64-setup.exe", "browser_download_url": "https://example.test/setup.exe"},
            {"name": "CodexPlusPlus_1.0.9_x64.dmg", "browser_download_url": "https://example.test/app.dmg"}
        ]
    }))
    .unwrap();

    assert_eq!(release.version, "v1.0.9");
    if cfg!(windows) {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus_1.0.9_x64-setup.exe")
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus_1.0.9_x64.dmg")
        );
    } else {
        assert_eq!(release.asset_name.as_deref(), None);
    }
}

#[test]
fn latest_json_payload_selects_platform_installer_without_github_api_shape() {
    let release = release_from_latest_json_payload(&json!({
        "version": "v1.1.6",
        "url": "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.1.6",
        "body": "静态更新描述",
        "assets": [
            {"name": "source.zip", "url": "https://example.test/source.zip"},
            {"name": "CodexPlusPlus-1.1.6-windows-x64-setup.exe", "url": "https://example.test/setup.exe"},
            {"name": "CodexPlusPlus-1.1.6-macos-x64.dmg", "url": "https://example.test/app.dmg"}
        ]
    }))
    .unwrap();

    assert_eq!(release.version, "v1.1.6");
    assert_eq!(release.body, "静态更新描述");
    if cfg!(windows) {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.1.6-windows-x64-setup.exe")
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.1.6-macos-x64.dmg")
        );
    } else {
        assert_eq!(release.asset_name.as_deref(), None);
    }
}

#[test]
fn asset_selection_prefers_current_platform_artifacts() {
    let assets = vec![
        (
            "CodexPlusPlus.zip".to_string(),
            "https://example.test/source.zip".to_string(),
        ),
        (
            "codex-plus-plus-manager.exe".to_string(),
            "https://example.test/manager.exe".to_string(),
        ),
        (
            "CodexPlusPlus_1.0.9_x64-setup.exe".to_string(),
            "https://example.test/setup.exe".to_string(),
        ),
        (
            "CodexPlusPlus_1.0.9_x64.dmg".to_string(),
            "https://example.test/app.dmg".to_string(),
        ),
    ];

    if cfg!(windows) {
        let selected = select_update_asset(&assets).unwrap();
        assert_eq!(selected.name, "CodexPlusPlus_1.0.9_x64-setup.exe");
    } else if cfg!(target_os = "macos") {
        let selected = select_update_asset(&assets).unwrap();
        assert_eq!(selected.name, "CodexPlusPlus_1.0.9_x64.dmg");
    } else {
        assert!(select_update_asset(&assets).is_none());
    }
}

#[test]
fn asset_selection_distinguishes_x64_and_arm64_macos_dmgs() {
    // Regression test for the bug where an x86_64 Mac user could be handed
    // the arm64 DMG (or vice versa) because `is_macos_installer_asset` did
    // not check the arch token in the filename.
    let assets = vec![
        (
            "CodexPlusPlus-1.2.17-macos-arm64.dmg".to_string(),
            "https://example.test/app-arm64.dmg".to_string(),
        ),
        (
            "CodexPlusPlus-1.2.17-macos-x64.dmg".to_string(),
            "https://example.test/app-x64.dmg".to_string(),
        ),
    ];

    if cfg!(target_os = "macos") {
        let selected = select_update_asset(&assets)
            .expect("a macOS DMG should be selected for the running arch");
        let expected = match std::env::consts::ARCH {
            "x86_64" => "CodexPlusPlus-1.2.17-macos-x64.dmg",
            "aarch64" => "CodexPlusPlus-1.2.17-macos-arm64.dmg",
            other => panic!("unexpected target arch in test: {other}"),
        };
        assert_eq!(
            selected.name, expected,
            "x86_64 binary must select x64 DMG, aarch64 binary must select arm64 DMG"
        );
    } else {
        // Non-macOS platforms should not pick either macOS DMG.
        assert!(select_update_asset(&assets).is_none());
    }
}

#[test]
fn safe_asset_name_rejects_path_traversal() {
    assert_eq!(safe_asset_name("pkg.zip").unwrap(), "pkg.zip");
    assert!(safe_asset_name("../pkg.zip").is_err());
    assert!(safe_asset_name("").is_err());
}

#[test]
fn download_asset_to_writes_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let release = Release {
        version: "v1.0.9".to_string(),
        url: "https://example.test".to_string(),
        body: "fixes".to_string(),
        asset_name: Some("pkg.zip".to_string()),
        asset_url: Some("https://example.test/pkg.zip".to_string()),
        asset_sha256: None,
    };

    let path = download_asset_to(&release, b"abcdef", dir.path()).unwrap();

    assert_eq!(path, dir.path().join("pkg.zip"));
    assert_eq!(std::fs::read(path).unwrap(), b"abcdef");
}

#[test]
fn latest_json_payload_selects_linux_portable_zip_with_digest() {
    let release = release_from_latest_json_payload(&json!({
        "version": "v1.2.43",
        "url": "https://github.com/BigPizzaV3/CodexPlusPlus/releases/tag/v1.2.43",
        "body": "linux support",
        "assets": [
            {"name": "source.zip", "url": "https://example.test/source.zip"},
            {"name": "CodexPlusPlus-1.2.43-windows-x64-setup.exe", "url": "https://example.test/setup.exe"},
            {
                "name": "CodexPlusPlus-1.2.43-linux-x64.zip",
                "url": "https://example.test/linux.zip",
                "sha256": "AABBCC"
            }
        ]
    }))
    .unwrap();

    if cfg!(target_os = "linux") && std::env::consts::ARCH == "x86_64" {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.2.43-linux-x64.zip")
        );
        assert_eq!(release.asset_sha256.as_deref(), Some("AABBCC"));
    } else if cfg!(windows) {
        assert_eq!(
            release.asset_name.as_deref(),
            Some("CodexPlusPlus-1.2.43-windows-x64-setup.exe")
        );
    }
}

#[test]
fn github_payload_extracts_sha256_from_digest_field() {
    let release = release_from_github_payload(&json!({
        "tag_name": "v1.2.43",
        "html_url": "https://example.test/release",
        "body": "notes",
        "assets": [
            {
                "name": "CodexPlusPlus-1.2.43-linux-x64.zip",
                "browser_download_url": "https://example.test/linux.zip",
                "digest": "sha256:00112233"
            }
        ]
    }))
    .unwrap();

    if cfg!(target_os = "linux") && std::env::consts::ARCH == "x86_64" {
        assert_eq!(release.asset_sha256.as_deref(), Some("00112233"));
    } else {
        assert_eq!(release.asset_name.as_deref(), None);
    }
}

#[cfg(target_os = "linux")]
mod linux_archive {
    use codex_plus_core::install::linux::{
        self, INSTALL_MANAGED_BY, INSTALL_MANIFEST_FILE, LinuxInstallRoots,
    };
    use sha2::Digest;
    use std::io::Write;

    fn build_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn manifest_bytes() -> Vec<u8> {
        serde_json::json!({
            "managedBy": INSTALL_MANAGED_BY,
            "version": "9.9.9",
            "files": [
                "bin/codex-plus-plus",
                "bin/codex-plus-plus-manager",
                "share/icon.png",
            ],
        })
        .to_string()
        .into_bytes()
    }

    fn valid_archive() -> Vec<u8> {
        build_archive(&[
            (INSTALL_MANIFEST_FILE, &manifest_bytes()),
            ("bin/codex-plus-plus", b"launcher-v9"),
            ("bin/codex-plus-plus-manager", b"manager-v9"),
            ("share/icon.png", b"icon"),
        ])
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn temp_roots(temp: &tempfile::TempDir) -> LinuxInstallRoots {
        LinuxInstallRoots {
            applications_dir: temp.path().join("share/applications"),
            icons_dir: temp.path().join("share/icons"),
            library_root: temp.path().join("lib/codex-plus-plus"),
        }
    }

    #[test]
    fn update_archive_installs_and_activates_new_version() {
        let temp = tempfile::tempdir().unwrap();
        let roots = temp_roots(&temp);
        let archive = valid_archive();

        let installed =
            linux::install_update_archive(&archive, &sha256_hex(&archive), &roots, "9.9.9")
                .unwrap();

        let current = roots.library_root.join("current");
        assert_eq!(
            std::fs::read_link(&current).unwrap(),
            std::path::PathBuf::from("versions/9.9.9")
        );
        assert_eq!(
            std::fs::read_to_string(current.join("bin/codex-plus-plus")).unwrap(),
            "launcher-v9"
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(installed.version_dir.join("bin/codex-plus-plus"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }

    #[test]
    fn update_archive_rejects_bad_digest_and_leaves_no_version() {
        let temp = tempfile::tempdir().unwrap();
        let roots = temp_roots(&temp);
        let archive = valid_archive();

        let error = linux::install_update_archive(&archive, "deadbeef", &roots, "9.9.9")
            .unwrap_err()
            .to_string();
        assert!(error.contains("SHA-256"), "unexpected error: {error}");
        assert!(!roots.library_root.join("versions/9.9.9").exists());
    }

    #[test]
    fn update_archive_rejects_traversal_undeclared_and_duplicate_entries() {
        let temp = tempfile::tempdir().unwrap();
        let roots = temp_roots(&temp);

        let traversal = build_archive(&[
            (INSTALL_MANIFEST_FILE, &manifest_bytes()),
            ("bin/codex-plus-plus", b"x"),
            ("bin/codex-plus-plus-manager", b"x"),
            ("../escape", b"nope"),
        ]);
        let error =
            linux::install_update_archive(&traversal, &sha256_hex(&traversal), &roots, "9.9.9")
                .unwrap_err()
                .to_string();
        assert!(error.contains("路径非法"), "unexpected error: {error}");

        let undeclared = build_archive(&[
            (INSTALL_MANIFEST_FILE, &manifest_bytes()),
            ("bin/codex-plus-plus", b"x"),
            ("bin/codex-plus-plus-manager", b"x"),
            ("bin/extra-tool", b"nope"),
        ]);
        let error =
            linux::install_update_archive(&undeclared, &sha256_hex(&undeclared), &roots, "9.9.9")
                .unwrap_err()
                .to_string();
        assert!(error.contains("manifest"), "unexpected error: {error}");

        // 重复目标的防御在解压端保留，但 ZipWriter 本身拒绝写入重名条目，
        // 无法在此构造样本，因此不单独覆盖。

        assert!(!roots.library_root.join("versions/9.9.9").exists());
    }

    #[test]
    fn update_archive_rejects_missing_manifest_marker() {
        let temp = tempfile::tempdir().unwrap();
        let roots = temp_roots(&temp);
        let unmanaged = build_archive(&[
            (
                INSTALL_MANIFEST_FILE,
                br#"{"managedBy":"someone else","files":[]}"# as &[u8],
            ),
            ("bin/codex-plus-plus", b"x"),
        ]);

        let error =
            linux::install_update_archive(&unmanaged, &sha256_hex(&unmanaged), &roots, "9.9.9")
                .unwrap_err()
                .to_string();
        assert!(error.contains("管理标记"), "unexpected error: {error}");
    }

    #[test]
    fn rollback_restores_previous_version_after_update() {
        let temp = tempfile::tempdir().unwrap();
        let roots = temp_roots(&temp);

        // 先装入 1.0.0，再用更新包升到 9.9.9，回滚后 current 指回 1.0.0。
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("codex-plus-plus"), "old-launcher").unwrap();
        std::fs::write(source.join("codex-plus-plus-manager"), "old-manager").unwrap();
        let options = codex_plus_core::install::InstallOptions {
            install_root: None,
            launcher_path: Some(source.join("codex-plus-plus")),
            manager_path: Some(source.join("codex-plus-plus-manager")),
            remove_owned_data: false,
        };
        linux::install_user_scoped(&options, &roots, "1.0.0").unwrap();

        let archive = valid_archive();
        linux::install_update_archive(&archive, &sha256_hex(&archive), &roots, "9.9.9").unwrap();
        let current = roots.library_root.join("current");
        assert_eq!(
            std::fs::read_link(&current).unwrap(),
            std::path::PathBuf::from("versions/9.9.9")
        );

        linux::rollback_update(&roots.library_root).unwrap();
        assert_eq!(
            std::fs::read_link(&current).unwrap(),
            std::path::PathBuf::from("versions/1.0.0")
        );
        assert_eq!(
            std::fs::read_to_string(current.join("bin/codex-plus-plus")).unwrap(),
            "old-launcher"
        );
        // 回滚信息一次性使用。
        assert!(linux::rollback_update(&roots.library_root).is_err());
    }
}

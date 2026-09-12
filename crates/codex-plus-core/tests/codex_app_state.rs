use codex_plus_core::codex_app_state::{
    capture_app_state_snapshot, sync_app_state_after_provider_switch,
};
use serde_json::{Value, json};

#[test]
fn app_state_sync_restores_safe_state_and_ignores_sensitive_snapshot_keys() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    let mut initial_state = json!({
        "electron-saved-workspace-roots": ["C:/work/app", "C:\\work\\app\\"],
        "project-order": ["C:/work/app"],
        "active-workspace-roots": "C:/work/app",
        "electron-workspace-root-labels": {
            "C:/work/app/": "App"
        },
        "electron-avatar-overlay-bounds": {
            "x": 20,
            "y": 30,
            "width": 320,
            "height": 240
        },
        "electron-avatar-overlay-open": true,
        "electron-main-window-bounds": {
            "x": 10,
            "y": 10,
            "width": 1280,
            "height": 800
        },
        "thread-workspace-root-hints": {
            "thread-1": "C:/work/app",
            "local:thread-2": {
                "workspaceRoot": "D:/work/other"
            }
        },
        "thread-projectless-output-directories": {
            "thread-1": "C:/work/app/out"
        },
        "thread-writable-roots": {
            "thread-1": ["C:/work/app"]
        },
        "projectless-thread-ids": ["thread-1", "thread-1"],
        "electron-persisted-atom-state": {
            "app-shell:right-panel-width:v2:/": 420,
            "avatar-overlay-mascot-width-px": 160,
            "composer-auto-context-enabled": false,
            "default-service-tier": "priority",
            "diff-filter": "all",
            "enter-behavior": "cmdAlways",
            "first-awake-pet-notification-avatar-ids": ["otter"],
            "has-seen-multi-agent-composer-banner": true,
            "electron:onboarding-workspace-autolaunch-applied": true,
            "sidebar-collapsed-sections-v1": ["cloud"],
            "sidebar-project-expanded-v1-codex:C:/work/app": true,
            "sidebar-width": 296,
            "thread-summary-panel-section-expanded-progress": false,
            "thread-client-id-v1:thread-1": "do-not-copy",
            "heartbeat-thread-permissions-by-id": {
                "thread-1": "do-not-copy"
            },
            "prompt-history": ["secret"],
            "OPENAI_API_KEY": "do-not-copy",
            "provider-token-cache": "do-not-copy"
        },
        "prompt-history": ["secret"],
        "provider-token-cache": "secret"
    });
    if !cfg!(windows) {
        initial_state["electron-saved-workspace-roots"] = json!(["/work/app", "/work/app/"]);
        initial_state["project-order"] = json!(["/work/app"]);
        initial_state["active-workspace-roots"] = json!("/work/app");
        initial_state["electron-workspace-root-labels"] = json!({"/work/app/": "App"});
        initial_state["thread-workspace-root-hints"] = json!({
            "thread-1": "/work/app",
            "local:thread-2": {"workspaceRoot": "/work/other"}
        });
        initial_state["thread-projectless-output-directories"] =
            json!({"thread-1": "/work/app/out"});
        initial_state["thread-writable-roots"] = json!({"thread-1": ["/work/app"]});
    }
    std::fs::write(&state_path, initial_state.to_string()).unwrap();

    let snapshot_path = capture_app_state_snapshot(home)
        .unwrap()
        .expect("snapshot should be created");
    assert!(snapshot_path.is_file());
    let snapshot: Value =
        serde_json::from_str(&std::fs::read_to_string(&snapshot_path).unwrap()).unwrap();
    assert!(snapshot["state"].get("active-workspace-roots").is_none());

    let fresh_path = if cfg!(windows) {
        "D:/fresh/app"
    } else {
        "/fresh/app"
    };
    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": [fresh_path],
            "active-workspace-roots": fresh_path,
            "thread-workspace-root-hints": {"thread-3": fresh_path},
            "electron-persisted-atom-state": {"service-tier-default": "standard"}
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();

    assert!(result.changed);
    assert!(result.backup_path.as_deref().unwrap().is_dir());
    assert!(result.snapshot_path.as_deref().unwrap().is_file());
    assert_eq!(
        state["electron-saved-workspace-roots"],
        if cfg!(windows) {
            json!(["D:\\fresh\\app", "C:\\work\\app"])
        } else {
            json!(["/fresh/app", "/work/app"])
        }
    );
    assert_eq!(
        state["active-workspace-roots"],
        if cfg!(windows) {
            json!("D:\\fresh\\app")
        } else {
            json!("/fresh/app")
        }
    );
    assert_eq!(
        state["electron-workspace-root-labels"],
        if cfg!(windows) {
            json!({"C:\\work\\app": "App"})
        } else {
            json!({"/work/app": "App"})
        }
    );
    assert_eq!(state["electron-avatar-overlay-open"], true);
    assert_eq!(state["electron-avatar-overlay-bounds"]["width"], 320);
    assert_eq!(state["electron-main-window-bounds"]["height"], 800);
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-1"],
        if cfg!(windows) {
            "C:\\work\\app"
        } else {
            "/work/app"
        }
    );
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-3"],
        if cfg!(windows) {
            "D:\\fresh\\app"
        } else {
            fresh_path
        }
    );
    assert_eq!(
        state["thread-projectless-output-directories"]["thread-1"],
        if cfg!(windows) {
            "C:\\work\\app\\out"
        } else {
            "/work/app/out"
        }
    );
    assert_eq!(
        state["thread-writable-roots"]["thread-1"],
        if cfg!(windows) {
            json!(["C:\\work\\app"])
        } else {
            json!(["/work/app"])
        }
    );
    assert_eq!(state["projectless-thread-ids"], json!(["thread-1"]));
    assert_eq!(
        state["electron-persisted-atom-state"]["default-service-tier"],
        "priority"
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["composer-auto-context-enabled"],
        false
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["enter-behavior"],
        "cmdAlways"
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["avatar-overlay-mascot-width-px"],
        160
    );
    assert_eq!(state["electron-persisted-atom-state"]["sidebar-width"], 296);
    assert_eq!(
        state["electron-persisted-atom-state"]["app-shell:right-panel-width:v2:/"],
        420
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["sidebar-project-expanded-v1-codex:C:/work/app"],
        true
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["has-seen-multi-agent-composer-banner"],
        true
    );
    assert_eq!(
        state["electron-persisted-atom-state"]["electron:onboarding-workspace-autolaunch-applied"],
        true
    );
    assert!(state.get("prompt-history").is_none());
    assert!(state.get("provider-token-cache").is_none());
    assert!(
        state["electron-persisted-atom-state"]
            .get("prompt-history")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("thread-client-id-v1:thread-1")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("heartbeat-thread-permissions-by-id")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("OPENAI_API_KEY")
            .is_none()
    );
    assert!(
        state["electron-persisted-atom-state"]
            .get("provider-token-cache")
            .is_none()
    );
}

#[test]
fn app_state_sync_normalizes_current_state_and_writes_backup_before_change() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let state_path = home.join(".codex-global-state.json");
    let original_saved_roots = if cfg!(windows) {
        json!(["C:/work/app", "C:\\work\\app\\"])
    } else {
        json!(["/work/app", "/work/app/"])
    };
    let original_active_root = if cfg!(windows) {
        "C:/work/app/"
    } else {
        "/work/app/"
    };
    std::fs::write(
        &state_path,
        json!({
            "electron-saved-workspace-roots": original_saved_roots,
            "active-workspace-roots": original_active_root,
            "projectless-thread-ids": ["thread-1", "thread-1"]
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let backup_path = result
        .backup_path
        .expect("normalization should create backup");
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    let backup: Value = serde_json::from_str(
        &std::fs::read_to_string(backup_path.join(".codex-global-state.json")).unwrap(),
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(
        state["electron-saved-workspace-roots"],
        if cfg!(windows) {
            json!(["C:\\work\\app"])
        } else {
            json!(["/work/app"])
        }
    );
    assert_eq!(
        state["active-workspace-roots"],
        if cfg!(windows) {
            json!("C:\\work\\app")
        } else {
            json!("/work/app")
        }
    );
    assert_eq!(state["projectless-thread-ids"], json!(["thread-1"]));
    assert_eq!(
        backup["electron-saved-workspace-roots"],
        original_saved_roots
    );
}

#[cfg(not(windows))]
#[test]
fn app_state_sync_repairs_legacy_linux_project_paths() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::write(
        home.join(".codex-global-state.json"),
        json!({
            "electron-saved-workspace-roots": [r"\home\Zyphorix\Documents\App"],
            "project-order": [r"\home\Zyphorix\Documents\App"],
            "electron-workspace-root-labels": {
                r"\home\Zyphorix\Documents\App": "App"
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".codex-global-state.json")).unwrap(),
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(
        state["electron-saved-workspace-roots"],
        json!(["/home/Zyphorix/Documents/App"])
    );
    assert_eq!(
        state["project-order"],
        json!(["/home/Zyphorix/Documents/App"])
    );
    assert_eq!(
        state["electron-workspace-root-labels"],
        json!({"/home/Zyphorix/Documents/App": "App"})
    );
    assert!(result.backup_path.unwrap().is_dir());
}

#[cfg(not(windows))]
#[test]
fn app_state_sync_repairs_known_thread_workspace_paths() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::write(
        home.join(".codex-global-state.json"),
        json!({
            "thread-workspace-root-hints": {
                "thread-1": r"\data\Projects\One",
                "thread-2": {
                    "workspaceRoot": r"\home\Zyphorix\Two",
                    "keep": true
                }
            },
            "thread-projectless-output-directories": {
                "thread-1": r"\data\Projects\One\out"
            },
            "thread-writable-roots": {
                "thread-1": [r"\data\Projects\One", r"\data\Projects\One"]
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = sync_app_state_after_provider_switch(home).unwrap();
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".codex-global-state.json")).unwrap(),
    )
    .unwrap();

    assert!(result.changed);
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-1"],
        "/data/Projects/One"
    );
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-2"]["workspaceRoot"],
        "/home/Zyphorix/Two"
    );
    assert_eq!(
        state["thread-workspace-root-hints"]["thread-2"]["keep"],
        true
    );
    assert_eq!(
        state["thread-projectless-output-directories"]["thread-1"],
        "/data/Projects/One/out"
    );
    assert_eq!(
        state["thread-writable-roots"]["thread-1"],
        json!(["/data/Projects/One"])
    );
}

# Linux Codex Project State Sync Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve Codex's saved project list on Linux, repair legacy backslash-corrupted paths, and prevent historical snapshots from reactivating an old project.

**Architecture:** Keep `~/.codex/.codex-global-state.json` as the only local project catalog. Add platform-aware pure path normalization inside `codex_app_state`, normalize only the known project-bearing fields, and exclude `active-workspace-roots` from snapshot capture and merge while retaining backup-first atomic writes.

**Tech Stack:** Rust 2024, `serde_json`, `anyhow`, `tempfile`, Cargo integration tests.

## Global Constraints

- Codex `.codex-global-state.json` is the only source for the local project list.
- Do not derive saved projects from manager session `cwd` values or Zed remote project records.
- Preserve saved roots, ordering, labels, and thread workspace metadata.
- Never restore `active-workspace-roots` from a historical snapshot.
- Preserve existing Windows drive and UNC behavior.
- Create a recoverable backup before modifying live user state.
- Do not expose authentication data in tests, diagnostics, or command output.

---

### Task 1: Platform-aware workspace path normalization

**Files:**
- Modify: `crates/codex-plus-core/src/codex_app_state.rs`
- Test: `crates/codex-plus-core/src/codex_app_state.rs`
- Test: `crates/codex-plus-core/tests/codex_app_state.rs`

**Interfaces:**
- Consumes: JSON string paths from Codex project state.
- Produces: private `DesktopPathStyle`, `normalize_desktop_path_for_style(value, style) -> Option<String>`, and style-aware path deduplication used by the existing public snapshot/sync functions.

- [ ] **Step 1: Add failing pure-function tests for Windows and Unix behavior**

Add a `#[cfg(test)] mod path_tests` at the end of `codex_app_state.rs`:

```rust
#[cfg(test)]
mod path_tests {
    use super::{DesktopPathStyle, dedupe_paths_for_style, normalize_desktop_path_for_style};

    #[test]
    fn unix_paths_keep_forward_slashes_and_repair_legacy_backslashes() {
        assert_eq!(
            normalize_desktop_path_for_style("/data/Projects/App/", DesktopPathStyle::Unix),
            Some("/data/Projects/App".to_string())
        );
        assert_eq!(
            normalize_desktop_path_for_style(r"\home\Zyphorix\Documents\App", DesktopPathStyle::Unix),
            Some("/home/Zyphorix/Documents/App".to_string())
        );
    }

    #[test]
    fn unix_deduplication_is_case_sensitive() {
        assert_eq!(
            dedupe_paths_for_style(
                vec!["/work/App".into(), "/work/app".into(), "/work/App/".into()],
                DesktopPathStyle::Unix,
            ),
            vec!["/work/App", "/work/app"]
        );
    }

    #[test]
    fn windows_paths_keep_drive_and_unc_semantics() {
        assert_eq!(
            normalize_desktop_path_for_style("C:/work/app/", DesktopPathStyle::Windows),
            Some(r"C:\work\app".to_string())
        );
        assert_eq!(
            dedupe_paths_for_style(
                vec!["C:/work/App".into(), r"C:\work\app\".into()],
                DesktopPathStyle::Windows,
            ),
            vec![r"C:\work\App"]
        );
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core --lib path_tests
```

Expected: compilation fails because `DesktopPathStyle`, `dedupe_paths_for_style`, and `normalize_desktop_path_for_style` do not exist.

- [ ] **Step 3: Implement the minimal platform-aware normalizer**

Replace the unconditional path conversion with this structure:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopPathStyle {
    Windows,
    Unix,
}

fn current_desktop_path_style() -> DesktopPathStyle {
    if cfg!(windows) {
        DesktopPathStyle::Windows
    } else {
        DesktopPathStyle::Unix
    }
}

fn normalize_desktop_path(value: &str) -> Option<String> {
    normalize_desktop_path_for_style(value, current_desktop_path_style())
}

fn normalize_desktop_path_for_style(
    value: &str,
    style: DesktopPathStyle,
) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    match style {
        DesktopPathStyle::Windows => {
            let mut path = trimmed.replace('/', r"\");
            while path.len() > 3 && path.ends_with('\\') {
                path.pop();
            }
            Some(path)
        }
        DesktopPathStyle::Unix => {
            let mut path = if trimmed.starts_with('\\')
                && !trimmed.starts_with(r"\\")
                && !trimmed.contains('/')
            {
                trimmed.replace('\\', "/")
            } else {
                trimmed.to_string()
            };
            while path.len() > 1 && path.ends_with('/') {
                path.pop();
            }
            Some(path)
        }
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    dedupe_paths_for_style(paths, current_desktop_path_style())
}

fn dedupe_paths_for_style(
    paths: Vec<String>,
    style: DesktopPathStyle,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for raw in paths {
        let Some(path) = normalize_desktop_path_for_style(&raw, style) else {
            continue;
        };
        let comparable = match style {
            DesktopPathStyle::Windows => path.to_ascii_lowercase(),
            DesktopPathStyle::Unix => path.clone(),
        };
        if seen.insert(comparable) {
            result.push(path);
        }
    }
    result
}
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
rtk cargo test -p codex-plus-core --lib path_tests
```

Expected: all three `path_tests` pass.

- [ ] **Step 5: Add an integration regression for live Linux state**

In `crates/codex-plus-core/tests/codex_app_state.rs`, add a Unix-only test that writes corrupted saved roots and runs the real public sync API:

```rust
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
    assert_eq!(state["electron-saved-workspace-roots"], json!(["/home/Zyphorix/Documents/App"]));
    assert_eq!(state["project-order"], json!(["/home/Zyphorix/Documents/App"]));
    assert_eq!(state["electron-workspace-root-labels"], json!({"/home/Zyphorix/Documents/App": "App"}));
    assert!(result.backup_path.unwrap().is_dir());
}
```

- [ ] **Step 6: Run the integration test, then the full app-state test target**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state app_state_sync_repairs_legacy_linux_project_paths
rtk cargo test -p codex-plus-core --test codex_app_state
```

Expected: the new test passes; any older Windows-shaped assertions fail and identify the fixtures that must become platform-neutral before Task 1 is complete. Update those fixtures to use native Unix paths under `#[cfg(not(windows))]` and retain equivalent Windows expectations under `#[cfg(windows)]`; do not weaken behavioral assertions.

- [ ] **Step 7: Commit Task 1**

```bash
rtk git add crates/codex-plus-core/src/codex_app_state.rs crates/codex-plus-core/tests/codex_app_state.rs
rtk git commit -m "fix: preserve native Codex project paths"
```

---

### Task 2: Separate saved projects from transient active workspace state

**Files:**
- Modify: `crates/codex-plus-core/src/codex_app_state.rs`
- Test: `crates/codex-plus-core/tests/codex_app_state.rs`

**Interfaces:**
- Consumes: snapshots created by `capture_app_state_snapshot` and legacy snapshots that may contain `active-workspace-roots`.
- Produces: `safe_snapshot_from_state` output without active roots and `merge_safe_snapshot` behavior that ignores legacy active roots.

- [ ] **Step 1: Write the failing snapshot test**

Update `app_state_sync_restores_safe_state_and_ignores_sensitive_snapshot_keys` so the current state contains only `/fresh/app` as active while the captured snapshot contains `/work/app`, then assert:

```rust
assert_eq!(state["active-workspace-roots"], json!(["/fresh/app"]));
assert_eq!(
    state["electron-saved-workspace-roots"],
    json!(["/fresh/app", "/work/app"])
);
```

Add a direct snapshot-content assertion after capture:

```rust
let snapshot: Value = serde_json::from_str(
    &std::fs::read_to_string(&snapshot_path).unwrap(),
)
.unwrap();
assert!(snapshot["state"].get("active-workspace-roots").is_none());
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state app_state_sync_restores_safe_state_and_ignores_sensitive_snapshot_keys
```

Expected: FAIL because the snapshot contains the old active root and merge produces both active roots.

- [ ] **Step 3: Remove active roots from capture and merge**

In `safe_snapshot_from_state`, remove the block that inserts `ACTIVE_WORKSPACE_ROOTS_KEY`. In `merge_safe_snapshot`, remove the block that reads active paths from target and snapshot and writes a merged active value. Keep `normalize_current_state` handling of the current live active value so legacy separator corruption can still be repaired without importing historical activity.

- [ ] **Step 4: Run the focused and full tests**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state app_state_sync_restores_safe_state_and_ignores_sensitive_snapshot_keys
rtk cargo test -p codex-plus-core --test codex_app_state
```

Expected: all `codex_app_state` tests pass, including projectless preparation preserving saved roots and clearing only the live active root when explicitly requested.

- [ ] **Step 5: Commit Task 2**

```bash
rtk git add crates/codex-plus-core/src/codex_app_state.rs crates/codex-plus-core/tests/codex_app_state.rs
rtk git commit -m "fix: keep active workspace out of project snapshots"
```

---

### Task 3: Normalize known thread workspace metadata without inventing projects

**Files:**
- Modify: `crates/codex-plus-core/src/codex_app_state.rs`
- Test: `crates/codex-plus-core/tests/codex_app_state.rs`

**Interfaces:**
- Consumes: the three known thread state maps named by `THREAD_STATE_MAP_KEYS`.
- Produces: normalized path values for string hints, `{ "workspaceRoot": ... }` hints, output directories, and writable-root arrays; unknown object fields remain byte-for-byte equivalent at the JSON value level.

- [ ] **Step 1: Add a failing Unix integration test**

Add `#[cfg(not(windows))] fn app_state_sync_repairs_known_thread_workspace_paths()` with state containing:

```rust
json!({
    "thread-workspace-root-hints": {
        "thread-1": r"\data\Projects\One",
        "thread-2": {"workspaceRoot": r"\home\Zyphorix\Two", "keep": true}
    },
    "thread-projectless-output-directories": {
        "thread-1": r"\data\Projects\One\out"
    },
    "thread-writable-roots": {
        "thread-1": [r"\data\Projects\One", r"\data\Projects\One"]
    }
})
```

Assert string values become `/data/Projects/One`, `/home/Zyphorix/Two`, and `/data/Projects/One/out`; writable roots are deduplicated; `keep` remains `true`.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state app_state_sync_repairs_known_thread_workspace_paths
```

Expected: FAIL because current code trims thread IDs but clones their path values unchanged.

- [ ] **Step 3: Implement field-specific thread value normalization**

Replace generic `normalize_string_keyed_map` calls for `THREAD_STATE_MAP_KEYS` with a helper that switches on the known key:

```rust
fn normalize_thread_state_map(key: &str, map: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    for (thread_id, value) in map {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            continue;
        }
        let value = match key {
            "thread-workspace-root-hints" => normalize_workspace_hint(value),
            "thread-projectless-output-directories" => normalize_path_string(value),
            "thread-writable-roots" => Value::Array(
                dedupe_paths(path_array(value))
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
            _ => value.clone(),
        };
        next.insert(thread_id.to_string(), value);
    }
    next
}

fn normalize_path_string(value: &Value) -> Value {
    value
        .as_str()
        .and_then(normalize_desktop_path)
        .map(Value::String)
        .unwrap_or_else(|| value.clone())
}

fn normalize_workspace_hint(value: &Value) -> Value {
    if value.is_string() {
        return normalize_path_string(value);
    }
    let Some(mut object) = value.as_object().cloned() else {
        return value.clone();
    };
    if let Some(root) = object.get("workspaceRoot").cloned() {
        object.insert("workspaceRoot".to_string(), normalize_path_string(&root));
    }
    Value::Object(object)
}
```

Use the same helper in snapshot capture, current-state normalization, and snapshot merge so all three boundaries agree.

- [ ] **Step 4: Run tests and verify GREEN**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state app_state_sync_repairs_known_thread_workspace_paths
rtk cargo test -p codex-plus-core --test codex_app_state
```

Expected: all tests pass.

- [ ] **Step 5: Commit Task 3**

```bash
rtk git add crates/codex-plus-core/src/codex_app_state.rs crates/codex-plus-core/tests/codex_app_state.rs
rtk git commit -m "fix: repair Codex thread workspace metadata"
```

---

### Task 4: Repair the current Codex state and verify the installed launch path

**Files:**
- Modify at runtime with backup: `~/.codex/.codex-global-state.json`
- Verify: `~/.codex/.codex-global-state.json.bak`
- Verify: `~/.codex/backups_state/app-state-sync/<timestamp>/`
- Build outputs: `target/release/codex-plus-plus`, `target/release/codex-plus-plus-manager`

**Interfaces:**
- Consumes: the tested public `sync_app_state_after_provider_switch(&Path)` behavior through the normal Codex++ launch/provider-sync path.
- Produces: usable Unix saved roots in current Codex state and updated release binaries used by the existing desktop entries.

- [ ] **Step 1: Run package and workspace regression tests**

Run:

```bash
rtk cargo test -p codex-plus-core --test codex_app_state
rtk cargo test -p codex-plus-core
rtk cargo test -p codex-plus-data
rtk cargo test --workspace
```

Expected: every command exits 0 with no failing tests.

- [ ] **Step 2: Build the release launcher and manager**

Run:

```bash
rtk cargo build --release -p codex-plus-launcher -p codex-plus-manager
```

Expected: `target/release/codex-plus-plus` and `target/release/codex-plus-plus-manager` are rebuilt successfully.

- [ ] **Step 3: Capture a pre-repair state summary without secrets**

Read and report only these keys: `electron-saved-workspace-roots`, `project-order`, `active-workspace-roots`, `electron-workspace-root-labels`, and the three known thread workspace maps. Confirm the current invalid `\home\...` or `\data\...` entries are present before migration.

- [ ] **Step 4: Trigger one normal Codex++ state synchronization**

Run the rebuilt launcher through its existing desktop-entry executable path. The launcher captures a snapshot, performs the configured provider sync, invokes `sync_app_state_after_provider_switch`, and starts Codex. Close the launched Codex window after confirming the project selector loads; do not open a project automatically.

Expected: a new `codex_app_state.synced` diagnostic event records the changed project keys and a backup path.

- [ ] **Step 5: Verify repaired live state and recovery artifacts**

Read the same non-sensitive keys as Step 3 and assert:

- saved roots and project order use `/home/...` or `/data/...`;
- no saved local root begins with a single backslash;
- historical projects remain selectable;
- active roots were not imported from the snapshot;
- `.codex-global-state.json.bak` exists;
- the logged timestamped backup contains the pre-repair JSON.

- [ ] **Step 6: Verify repository scope**

Run:

```bash
rtk git status --short
rtk git diff --check HEAD~3..HEAD
```

Expected: only the planned source/test commits plus the user's pre-existing Linux full-stack edits are present; `git diff --check` reports no whitespace errors.


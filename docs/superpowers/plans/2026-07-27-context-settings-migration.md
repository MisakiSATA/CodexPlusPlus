# Context Settings Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor Codex++ context migration into an internal module, preserve global tool/plugin semantics, and install the verified build into the current Linux user installation with rollback protection.

**Architecture:** `settings.rs` remains the settings-store orchestrator while a new `settings/context.rs` module owns extraction, merge precedence, selection synchronization, and minimal raw-JSON persistence. The renderer keeps its bridge fallback and treats the removed Codex marketplace signal bundle as an optional compatibility path.

**Tech Stack:** Rust 2024, serde/serde_json, toml_edit through the existing `relay_config` API, JavaScript renderer injection, Node.js tests, Vite, Cargo, Linux user-level portable installer.

## Global Constraints

- Do not read, print, migrate, or commit real API keys, `auth.json`, or complete user `config.toml` contents.
- Existing global `relayContextConfigContents` wins conflicts; legacy common/profile context only fills missing entries.
- Context normalization is idempotent and deletion of the last global entry remains deleted after reload.
- Incremental persistence may alter only profile context fields and must not serialize derived model/provider/API-key fields.
- The removed `app-server-manager-signals-*` asset is optional; the bridge fallback remains active.
- Do not push to the official upstream remote.
- Back up the current user installation before replacing version `1.2.42`.

---

### Task 1: Lock migration edge cases with failing tests

**Files:**
- Modify: `crates/codex-plus-core/src/settings.rs`

**Interfaces:**
- Consumes: `SettingsStore::update(Value) -> anyhow::Result<BackendSettings>`.
- Produces: regression contracts for parent-table extraction, global conflict precedence, persistence, and idempotence.

- [ ] **Step 1: Add a parent-table migration test**

Add a test using a legacy profile whose `configContents` contains both the parent table and a plugin child:

```rust
#[test]
fn settings_store_update_migrates_parent_context_tables() {
    let dir = temp_dir();
    let store = SettingsStore::new(dir.join("settings.json"));
    let updated = store.update(json!({
        "relayProfiles": [{
            "id": "relay-a",
            "name": "供应商 A",
            "relayMode": "pureApi",
            "configContents": "model = \"gpt-5.6\"\n\n[plugins]\n\n[plugins.\"browser@openai-bundled\"]\nenabled = true\n"
        }],
        "activeRelayId": "relay-a"
    })).unwrap();

    assert!(!updated.relay_profiles[0].config_contents.contains("[plugins]"));
    assert!(updated.relay_context_config_contents.contains("[plugins]"));
    assert!(updated.relay_context_config_contents.contains("[plugins.\"browser@openai-bundled\"]"));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core settings::tests::settings_store_update_migrates_parent_context_tables
```

Expected: FAIL because the existing table-header predicate leaves `[plugins]` in the profile config.

- [ ] **Step 3: Add global precedence and idempotence assertions**

Extend the context migration test so global `enabled = false` wins a legacy profile's `enabled = true`, then call `store.load()` twice and assert both loaded settings values are equal. Read the persisted JSON only to assert context field shapes; never print it.

- [ ] **Step 4: Run the focused settings suite**

Run:

```bash
rtk cargo test -p codex-plus-core settings::tests
```

Expected: the new parent-table test remains the only behavior failure before implementation.

### Task 2: Extract the context settings module and make tests green

**Files:**
- Create: `crates/codex-plus-core/src/settings/context.rs`
- Modify: `crates/codex-plus-core/src/settings.rs`

**Interfaces:**
- Produces: `pub(super) fn normalize(settings: &mut BackendSettings)`.
- Produces: `pub(super) fn persist_profile_fields(raw: &mut Map<String, Value>, profiles: &[RelayProfile])`.
- Consumes: `relay_config::merge_common_config_into_config`, `relay_config::list_context_entries_from_common_config`, and `relay_config::normalize_config_text`.

- [ ] **Step 1: Declare the internal module**

At the top of `settings.rs`, add:

```rust
mod context;
```

- [ ] **Step 2: Move context normalization behind one entry point**

Create `settings/context.rs` with this public-to-parent shape:

```rust
use serde_json::{Map, Value};

use super::{BackendSettings, RelayContextSelection, RelayProfile};

pub(super) fn normalize(settings: &mut BackendSettings) {
    let (common, legacy_common_context) = split_sections(&settings.relay_common_config_contents);
    let mut context = merge_sections(
        &settings.relay_context_config_contents,
        &legacy_common_context,
    );
    settings.relay_common_config_contents = normalize_text(common);

    for profile in &mut settings.relay_profiles {
        let (profile_config, legacy_profile_context) = split_sections(&profile.config_contents);
        profile.config_contents = profile_config;
        context = merge_sections(&context, &legacy_profile_context);
    }

    settings.relay_context_config_contents = normalize_text(context.clone());
    sync_profile_selections(settings, &context);
}

pub(super) fn persist_profile_fields(
    raw: &mut Map<String, Value>,
    profiles: &[RelayProfile],
) {
    // Match raw profiles by id, strip context tables from configContents,
    // and write only contextSelection/contextSelectionInitialized.
}
```

Keep parsing, merge, selection, and text-normalization helpers private to this module.

- [ ] **Step 3: Recognize parent context tables**

Use a predicate that covers parent and child headers without matching unrelated names:

```rust
fn is_context_table_header(header: &str) -> bool {
    ["mcp_servers", "skills", "plugins"].into_iter().any(|table| {
        header == format!("[{table}]") || header.starts_with(&format!("[{table}."))
    })
}
```

- [ ] **Step 4: Preserve global precedence**

When both sections are non-empty, call:

```rust
crate::relay_config::merge_common_config_into_config(incoming, current)
```

The legacy input is the target and the current global input is the source, so the existing merge implementation overwrites conflicts with global values.

- [ ] **Step 5: Keep empty-default and deletion semantics distinct**

Build `RelayContextSelection` from the final entries. Synchronize profiles only when the final selection is non-empty or any profile was already initialized. This leaves a pristine default profile uninitialized but clears initialized profiles after deletion of the last entry.

- [ ] **Step 6: Replace settings-store orchestration calls**

In `normalize_settings_config_sections`, call `context::normalize(&mut settings)` before `normalize_relay_profile_for_storage`. In `SettingsStore::update`, call `context::persist_profile_fields(&mut raw, &settings.relay_profiles)`. Remove the moved helpers from `settings.rs`.

- [ ] **Step 7: Run the focused tests and verify GREEN**

Run:

```bash
rtk cargo test -p codex-plus-core settings::tests
```

Expected: all settings tests pass, including parent-table migration, persistence, conflict priority, deletion, and default settings regression tests.

- [ ] **Step 8: Run format and diff checks for changed Rust files**

Run:

```bash
rtk rustfmt --edition 2024 --check crates/codex-plus-core/src/settings.rs crates/codex-plus-core/src/settings/context.rs
rtk git diff --check
```

Expected: both commands exit 0.

- [ ] **Step 9: Commit the module refactor**

```bash
rtk git add crates/codex-plus-core/src/settings.rs crates/codex-plus-core/src/settings/context.rs
rtk git commit -m "fix: migrate legacy context settings globally"
```

### Task 3: Preserve the optional plugin-marketplace compatibility path

**Files:**
- Modify: `assets/inject/renderer-inject.js`
- Modify: `crates/codex-plus-core/tests/cdp_bridge.rs`

**Interfaces:**
- Consumes: `loadOptionalCodexAppModule(namePart) -> Promise<object | null>`.
- Produces: a one-time `plugin_marketplace_request_patch_skipped` diagnostic with `reason: "asset_missing"`.

- [ ] **Step 1: Confirm the regression test detects the required call path**

The test must assert that `installPluginMarketplaceRequestPatch` uses:

```javascript
loadOptionalCodexAppModule("app-server-manager-signals-")
```

and does not use the required loader for that exact asset.

- [ ] **Step 2: Verify the established RED/GREEN record and current GREEN state**

The regression was already observed failing against the required loader and passing after the optional loader change. Re-run:

```bash
rtk cargo test -p codex-plus-core --test cdp_bridge injection_script_skips_removed_plugin_marketplace_asset_without_retrying
rtk cargo test -p codex-plus-core --test cdp_bridge
```

Expected: 1 focused test and the full CDP bridge test suite pass.

- [ ] **Step 3: Commit the renderer compatibility change**

```bash
rtk git add assets/inject/renderer-inject.js crates/codex-plus-core/tests/cdp_bridge.rs
rtk git commit -m "fix: skip removed marketplace signal asset"
```

### Task 4: Verify the complete source tree

**Files:**
- No source changes expected.

**Interfaces:**
- Consumes: all workspace crates and manager frontend scripts.
- Produces: fresh verification evidence for the exact commit to be installed.

- [ ] **Step 1: Run Rust workspace tests**

```bash
rtk cargo test --workspace
```

Expected: zero failures; currently configured ignored tests remain reported as ignored.

- [ ] **Step 2: Run manager tests, checks, and production build**

From `apps/codex-plus-manager` run:

```bash
rtk npm test
rtk npm run check
rtk npm run vite:build
```

Expected: unit tests pass, check prints `ok`, and Vite completes a production build.

- [ ] **Step 3: Run repository hygiene checks**

```bash
rtk git diff --check
rtk cargo fmt --all -- --check
```

Expected: task files are formatted. If the repository-wide formatter reports only the previously observed untouched lines in `src/install/linux.rs` and `tests/installers.rs`, record them as pre-existing rather than modifying unrelated code.

- [ ] **Step 4: Review the final diff and commit state**

```bash
rtk git status --short
rtk git log -3 --oneline --decorate
```

Expected: no uncommitted task changes remain and both implementation commits follow the design/plan commits.

### Task 5: Build, back up, install, and validate the live user installation

**Files:**
- Build output: `target/release/codex-plus-plus`
- Build output: `target/release/codex-plus-plus-manager`
- Package output: `dist/linux/app-x64/`
- Install target: `/home/Zyphorix/.local/lib/codex-plus-plus/`

**Interfaces:**
- Consumes: the project's release build and `scripts/installer/linux/package-portable.sh`.
- Produces: an atomically switched user installation and a timestamped rollback copy.

- [ ] **Step 1: Resolve and validate the current install target**

Use `readlink -f` on `/home/Zyphorix/.local/lib/codex-plus-plus/current`. Abort unless the resolved directory is exactly below `/home/Zyphorix/.local/lib/codex-plus-plus/versions/` and contains both expected binaries plus `install-manifest.json`.

- [ ] **Step 2: Create a non-overwriting rollback backup**

Copy the validated current version directory to a timestamped hidden directory below `/home/Zyphorix/.local/lib/codex-plus-plus/versions/`. Record its exact path and SHA-256 hashes for the two binaries. Do not delete previous backups.

- [ ] **Step 3: Build release binaries**

```bash
rtk cargo build --release
```

Expected: both launcher and manager release binaries exist and have a fresh modification timestamp.

- [ ] **Step 4: Build the portable package**

```bash
rtk bash scripts/installer/linux/package-portable.sh 1.2.42 x64
```

Expected: `dist/linux/CodexPlusPlus-1.2.42-linux-x64.zip`, its `.sha256`, and `dist/linux/app-x64/install.sh` exist; `sha256sum -c` succeeds.

- [ ] **Step 5: Install with the project installer**

Run the exact `dist/linux/app-x64/install.sh` after validating its manifest version is `1.2.42`. Expected: `current` atomically points to `versions/1.2.42`, desktop entries target `current/bin`, and installed binary hashes equal the staged package hashes.

- [ ] **Step 6: Start the installed manager and launcher**

Start `/home/Zyphorix/.local/lib/codex-plus-plus/current/bin/codex-plus-plus-manager` and confirm the process executable resolves beneath `current`. Then start the installed launcher and confirm its process and any injected Codex process use the new binary.

- [ ] **Step 7: Verify migrated context without exposing secrets**

Inspect only counts and IDs from the manager/settings response or local settings parser. Confirm global plugin/MCP/skill counts reflect legacy profile entries, every initialized profile selects those IDs, and no profile `configContents` retains context table headers. Do not print full configuration values.

- [ ] **Step 8: Verify runtime diagnostics**

After one refresh interval, search new diagnostics for event names only. Confirm no repeated `plugin_marketplace_request_patch_failed` entries for `app-server-manager-signals-`; either one `plugin_marketplace_request_patch_skipped` event or a successful legacy patch is acceptable.

- [ ] **Step 9: Roll back on any runtime failure**

If manager launch, launcher start, settings migration, or diagnostics validation fails, stop only the newly started processes and atomically repoint `current` to the recorded backup. Re-run the installed binary hash check and report the failure without deleting build artifacts or logs.

- [ ] **Step 10: Report the live result**

Report implementation commits, test totals, installed binary hashes, current symlink target, backup location, running-process state, context entry counts, and any pre-existing non-task verification warnings. Do not include secrets or complete settings/log contents.

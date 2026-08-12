# Linux ChatGPT Package Layout Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Codex++ launch and track the current Arch `openai-codex-desktop` package installed under `/usr/lib/chatgpt` without dropping compatibility with older Linux layouts.

**Architecture:** Extend the existing Linux path discovery candidate list and the existing `/proc` command-line classifier. Keep all launch behavior, path normalization, settings precedence, and lifecycle management unchanged.

**Tech Stack:** Rust 2024, Cargo integration tests, Tauri release binaries, Linux `/proc` process inspection.

## Global Constraints

- Prefer `/usr/lib/chatgpt`, then `/usr/lib/openai-codex-desktop`, then `/usr/lib/codex-plus-plus/app`.
- Do not modify `~/.codex` credentials, provider data, sessions, or the upstream `app.asar`.
- Do not create compatibility symlinks under `/usr/lib`.
- Preserve explicit and valid saved application-path precedence.

---

### Task 1: Current Linux application layout discovery

**Files:**
- Modify: `crates/codex-plus-core/src/app_paths.rs`
- Test: `crates/codex-plus-core/tests/launcher.rs`

**Interfaces:**
- Consumes: candidate application roots supplied by Linux default discovery.
- Produces: `find_linux_codex_app_default_from(candidates: &[PathBuf]) -> Option<PathBuf>` for deterministic tests and `find_linux_codex_app_default() -> Option<PathBuf>` for production defaults.

- [ ] **Step 1: Write a failing candidate-order regression test**

Add `find_linux_codex_app_default_from` to the Linux-only import and create
three temporary valid Linux desktop roots:

```rust
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
    }

    assert_eq!(
        find_linux_codex_app_default_from(&[
            current.clone(),
            previous.clone(),
            legacy.clone(),
        ])
        .as_deref(),
        Some(current.as_path())
    );

    std::fs::remove_file(current.join("ChatGPT")).unwrap();
    assert_eq!(
        find_linux_codex_app_default_from(&[current, previous.clone(), legacy])
            .as_deref(),
        Some(previous.as_path())
    );
}
```

- [ ] **Step 2: Verify RED**

Run `rtk cargo test -p codex-plus-core --test launcher app_paths_prefers_current_linux_chatgpt_package_layout`.
Expected: compilation failure because `find_linux_codex_app_default_from` does not exist.

- [ ] **Step 3: Implement the minimal discovery change**

Add the injected-candidate helper and make the production function pass the
three fixed roots in the required order:

```rust
#[cfg(target_os = "linux")]
pub fn find_linux_codex_app_default_from(candidates: &[PathBuf]) -> Option<PathBuf> {
    find_linux_codex_app(candidates)
}

#[cfg(target_os = "linux")]
pub fn find_linux_codex_app_default() -> Option<PathBuf> {
    find_linux_codex_app_default_from(&[
        PathBuf::from("/usr/lib/chatgpt"),
        PathBuf::from("/usr/lib/openai-codex-desktop"),
        PathBuf::from("/usr/lib/codex-plus-plus/app"),
    ])
}
```

- [ ] **Step 4: Verify GREEN**

Run the focused test and expect one passing test.

### Task 2: Current Linux process recognition

**Files:**
- Modify: `crates/codex-plus-core/src/watcher.rs`
- Test: `crates/codex-plus-core/tests/watcher.rs`

**Interfaces:**
- Consumes: null-separated Linux `/proc/<pid>/cmdline` bytes.
- Produces: the existing `find_linux_codex_processes_from_proc` result with current and previous Codex package layouts recognized.

- [ ] **Step 1: Write a failing `/usr/lib/chatgpt` process regression**

Expand the existing fixture to PIDs 120 through 124. Keep PID 120 as the old
main process and PID 121 as its renderer. Add these fixtures:

```rust
std::fs::write(
    proc_root.path().join("122/cmdline"),
    b"/usr/lib/chatgpt/ChatGPT\0--enable-sandbox\0/usr/lib/chatgpt/resources/app.asar\0",
)
.unwrap();
std::fs::write(
    proc_root.path().join("123/cmdline"),
    b"/usr/lib/chatgpt/ChatGPT\0--type=renderer\0--app-path=/usr/lib/chatgpt/resources/app.asar\0",
)
.unwrap();
std::fs::write(
    proc_root.path().join("124/cmdline"),
    b"/usr/lib/electron/electron\0/opt/another-app/resources/app.asar\0",
)
.unwrap();

assert_eq!(
    find_linux_codex_processes_from_proc(proc_root.path()),
    vec![120, 122]
);
```

- [ ] **Step 2: Verify RED**

Run `rtk cargo test -p codex-plus-core --test watcher linux_process_scan_matches_only_the_codex_electron_main_process`.
Expected: failure because the current-layout main PID is absent.

- [ ] **Step 3: Implement the minimal classifier change**

Accept `app.asar` arguments containing either supported package directory and
leave renderer filtering unchanged:

```rust
!argument.starts_with("--")
    && argument.ends_with("/resources/app.asar")
    && (argument.contains("/openai-codex-desktop/")
        || argument.contains("/chatgpt/"))
```

- [ ] **Step 4: Verify GREEN**

Run the focused watcher test and expect one passing test.

### Task 3: Regression, build, deploy, and runtime verification

**Files:**
- Generated: `target/release/codex-plus-plus`
- Generated: `target/release/codex-plus-plus-manager`
- Deploy: `~/.local/lib/codex-plus-plus/versions/1.2.42/bin/`

**Interfaces:**
- Consumes: passing core code and the existing user-level installation layout.
- Produces: updated user-level launch and manager binaries.

- [ ] **Step 1: Run focused and full tests**

Run `rtk cargo test -p codex-plus-core --test launcher`,
`rtk cargo test -p codex-plus-core --test watcher`, and
`rtk cargo test -p codex-plus-core`.

- [ ] **Step 2: Build release binaries**

Run `rtk cargo build --release -p codex-plus-launcher -p codex-plus-manager`.
Expected: exit code 0 and both required binaries in `target/release`.

- [ ] **Step 3: Stop stale user-level Codex++ processes and deploy**

Terminate only the known Codex++ manager, launcher, and legacy wrapper process
trees, then install the two release binaries into the existing 1.2.42 user
version directory. Preserve the existing files as timestamped backups.

- [ ] **Step 4: Verify a real launch**

Start the deployed launcher, verify diagnostics resolve `/usr/lib/chatgpt`,
confirm the ChatGPT process exists, query `http://127.0.0.1:9229/json/version`,
and query `http://127.0.0.1:57321/backend/status`.

- [ ] **Step 5: Review the final diff and installation state**

Run `rtk git diff --check`, inspect the scoped diff, and compare deployed
binary hashes with `target/release`.

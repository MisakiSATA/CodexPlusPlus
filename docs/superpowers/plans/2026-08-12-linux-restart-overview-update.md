# Linux Restart, Overview Cleanup, and Fork Update Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Linux Codex++ restart reliably replace the old launcher, remove the overview relay promotion, and source application updates from `MisakiSATA/CodexPlusPlus`.

**Architecture:** Extend the existing `/proc` watcher helpers so Linux can identify and stop only stale `codex-plus-plus` launchers while protecting the current process ancestry. Keep update repository ownership centralized in `update.rs`, and verify the UI cleanup with source-level Node tests that match the repository's existing frontend test style.

**Tech Stack:** Rust, Tokio/std process control, Tauri, React/TypeScript, Node test runner, Cargo.

## Global Constraints

- Do not modify or expose Codex credentials, `auth.json`, `.env`, or `config.toml` contents.
- Do not modify package-manager-owned files under `/usr/lib`.
- Keep theme, script, and advertisement repositories unchanged.
- Use TDD: every behavior change must be preceded by a failing test.
- Do not send signals to the active Codex CLI process.
- Real Desktop restart verification is allowed because Codex++ Desktop is currently stopped.

---

### Task 1: Linux launcher process control

**Files:**
- Modify: `crates/codex-plus-core/tests/watcher.rs`
- Modify: `crates/codex-plus-core/src/watcher.rs`

**Interfaces:**
- Consumes: Linux `/proc/<pid>/stat`, `/proc/<pid>/exe`, and `/proc/<pid>/cmdline` snapshots.
- Produces: `find_linux_launcher_processes_from_proc(proc_root: &Path, current_process_id: u32) -> Vec<u32>` and stricter Codex main-process filtering.

- [ ] **Step 1: Write failing watcher tests**

Add tests that construct a temporary proc tree and assert:

```rust
assert_eq!(
    find_linux_launcher_processes_from_proc(proc_root.path(), 30),
    vec![40]
);
```

The fixture must include protected ancestor PIDs `10 -> 20 -> 30`, a killable sibling launcher PID `40`, a manager PID `50`, and an unrelated executable. Extend the Codex process fixture with:

```rust
b"/usr/bin/gdb\0/usr/lib/chatgpt/ChatGPT\0"
```

and assert it is excluded.

- [ ] **Step 2: Verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core --test watcher
```

Expected: compilation/test failure because the Linux launcher scan does not exist and the gdb fixture is incorrectly matched.

- [ ] **Step 3: Implement minimal Linux scanning and stopping**

In `watcher.rs`:

- parse `/proc/<pid>/stat` for parent PID;
- resolve `/proc/<pid>/exe` and accept only basename `codex-plus-plus`;
- reuse `filter_killable_launcher_processes` with normalized Linux executable names;
- implement Linux `stop_launcher_processes()` and `stop_launcher_processes_and_wait()` using the scan;
- keep `SIGTERM` and the existing 5-second timeout;
- change `/usr/lib/chatgpt/ChatGPT` matching to `arguments.first() == Some(&...)` while preserving legacy `app.asar` matching.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
rtk cargo test -p codex-plus-core --test watcher
rtk cargo test -p codex-plus-core --test launcher
```

Expected: all watcher and launcher tests pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/codex-plus-core/src/watcher.rs crates/codex-plus-core/tests/watcher.rs
rtk git commit -m "fix: restart Linux launcher cleanly"
```

### Task 2: Remove overview relay promotion

**Files:**
- Create: `apps/codex-plus-manager/src/overview-source.test.ts`
- Modify: `apps/codex-plus-manager/src/App.tsx`
- Modify: `apps/codex-plus-manager/src/styles.css`
- Modify: `apps/codex-plus-manager/src/i18n-en.ts`

**Interfaces:**
- Consumes: `OverviewScreen` source and its dedicated CSS/i18n strings.
- Produces: overview starts with health information and contains no JOJO Code promotion.

- [ ] **Step 1: Write failing frontend source test**

Create a Node test that reads `App.tsx`, `styles.css`, and `i18n-en.ts`, isolates `OverviewScreen`, and asserts:

```ts
assert.doesNotMatch(overview, /JOJO Code|官方中转站|jojocode-overview/);
assert.doesNotMatch(styles, /\.jojocode-overview/);
assert.doesNotMatch(i18n, /Codex\+\+ 官方中转站|打开 JOJO Code/);
```

- [ ] **Step 2: Verify RED**

Run:

```bash
rtk npm test -- --test-name-pattern "overview"
```

from `apps/codex-plus-manager`.

Expected: failure because the promotion and dedicated styles/translations are present.

- [ ] **Step 3: Remove only promotion-owned code**

Delete the `jojocode-overview` panel from `OverviewScreen`, delete its CSS rules and responsive selectors, remove translation entries used only by that panel, and remove the `Network` icon import only if no remaining use requires it.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
rtk npm test
rtk npm run check
```

Expected: frontend tests and TypeScript checks pass.

- [ ] **Step 5: Commit**

```bash
rtk git add apps/codex-plus-manager/src/overview-source.test.ts apps/codex-plus-manager/src/App.tsx apps/codex-plus-manager/src/styles.css apps/codex-plus-manager/src/i18n-en.ts
rtk git commit -m "refactor: remove overview relay promotion"
```

### Task 3: Switch application update ownership to the fork

**Files:**
- Modify: `crates/codex-plus-core/tests/updater.rs`
- Modify: `crates/codex-plus-core/src/update.rs`
- Modify: `apps/codex-plus-manager/src/overview-source.test.ts`
- Modify: `apps/codex-plus-manager/src/App.tsx`

**Interfaces:**
- Consumes: `DEFAULT_REPOSITORY`, `DEFAULT_LATEST_JSON_URL`, and `AboutScreen` project links.
- Produces: update checks and visible repository links use `MisakiSATA/CodexPlusPlus`.

- [ ] **Step 1: Write failing repository ownership tests**

In `updater.rs`, import the constants and assert:

```rust
assert_eq!(DEFAULT_REPOSITORY, "MisakiSATA/CodexPlusPlus");
assert_eq!(
    DEFAULT_LATEST_JSON_URL,
    "https://github.com/MisakiSATA/CodexPlusPlus/releases/latest/download/latest.json"
);
```

In the frontend source test, isolate `AboutScreen` and require the fork homepage/issues URLs while rejecting `BigPizzaV3/CodexPlusPlus` in that section.

- [ ] **Step 2: Verify RED**

Run:

```bash
rtk cargo test -p codex-plus-core --test updater default_update_repository_targets_user_fork
rtk npm test -- --test-name-pattern "repository"
```

Expected: both fail against the upstream constants and About links.

- [ ] **Step 3: Implement minimal source changes**

Change only application update constants and AboutScreen homepage/issues/display text to `MisakiSATA/CodexPlusPlus`. Do not change theme market, script market, advertisements, Discord, or Telegram.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
rtk cargo test -p codex-plus-core --test updater
rtk npm test
rtk npm run check
```

Expected: all update and frontend tests pass.

- [ ] **Step 5: Commit**

```bash
rtk git add crates/codex-plus-core/src/update.rs crates/codex-plus-core/tests/updater.rs apps/codex-plus-manager/src/overview-source.test.ts apps/codex-plus-manager/src/App.tsx
rtk git commit -m "fix: source updates from user fork"
```

### Task 4: Full verification, build, deployment, and runtime acceptance

**Files:**
- Verify all modified files.
- Deploy user binaries under `/home/Zyphorix/.local/lib/codex-plus-plus/versions/1.2.42/bin/`.

**Interfaces:**
- Consumes: tested source tree and existing user-scoped install layout.
- Produces: deployed launcher/manager with verified Desktop restart behavior.

- [ ] **Step 1: Run complete verification**

```bash
rtk cargo test -p codex-plus-core
rtk npm test
rtk npm run check
rtk cargo fmt --check -p codex-plus-core -p codex-plus-manager
rtk git diff --check
```

Expected: all tests/checks pass, except any explicitly documented pre-existing repository-wide formatting differences outside changed files.

- [ ] **Step 2: Build release binaries**

```bash
CARGO_TARGET_DIR=/home/Zyphorix/.cache/codex-plus-plus-build rtk cargo build --release -p codex-plus-launcher -p codex-plus-manager
```

Expected: release build exits 0 and produces both binaries.

- [ ] **Step 3: Back up and deploy**

Copy the currently installed launcher and manager to timestamped `.pre-linux-restart-fix-*` backups, then install the new release binaries into the same user-level version directory. Compare SHA-256 hashes between build outputs and deployed files.

- [ ] **Step 4: Real Desktop restart acceptance**

Start Codex++ manager/launcher, record old launcher and ChatGPT PIDs, invoke the manager restart command through the UI or Tauri command path, then verify:

- old launcher exits;
- a new launcher PID owns `57320`;
- a new `/usr/lib/chatgpt/ChatGPT` main PID starts;
- `57321`, `9229`, and `9329` listen;
- `/backend/status` returns `status: ok` and version `1.2.42`;
- the manager overview has no JOJO promotion and About shows the fork repository.

- [ ] **Step 5: Commit any verification-only adjustments and push**

```bash
rtk git status --short
rtk git push origin codex/linux-full-stack
```

Expected: worktree clean and remote branch points at local HEAD.


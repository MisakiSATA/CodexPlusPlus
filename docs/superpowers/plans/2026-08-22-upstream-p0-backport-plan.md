# Upstream P0 Backport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Backport the selected upstream protocol, certificate, and DeepSeek catalog improvements into the Linux fork without disturbing Linux-only behavior.

**Architecture:** Keep protocol normalization in `protocol_proxy.rs`, system-root support in the workspace reqwest dependency declaration, metadata parsing and catalog entry merging in `model_suffix.rs`, and catalog activation decisions in `relay_config.rs`. Preserve existing model-window precedence and user-owned catalog pointers.

**Tech Stack:** Rust 2024 workspace, serde/serde_json, reqwest 0.12, tempfile-based Rust tests, JSON catalog assets.

## Global Constraints

- Do not merge or rebase `upstream/main`.
- Do not modify Linux installers, launcher behavior, project-state migration, or computer-use guard behavior.
- Preserve precedence: explicit per-model window, profile fallback window, metadata/default window.
- Preserve explicit external `model_catalog_json` pointers.
- Add tests before production behavior and run focused tests before broad tests.
- Do not bump the project version to `1.2.50`.

### Task 1: Protocol Proxy Robustness

**Files:**
- Modify: `crates/codex-plus-core/tests/protocol_proxy.rs`
- Modify: `crates/codex-plus-core/src/protocol_proxy.rs`

**Interfaces:**
- Existing `chat_completion_to_response`, `chat_sse_to_responses_sse`, and `responses_to_chat_completions` remain public behavior boundaries.
- Add only private helpers if needed; no new public API.

- [ ] **Step 1: Write failing usage-shape tests**

Add tests that convert a Chat completion with an empty `completion_tokens_details` object, one without that object, and an SSE completion without usage details. Each must assert `usage.output_tokens_details.reasoning_tokens == 0` in the generated Responses payload/event.

- [ ] **Step 2: Run the usage tests and verify failure**

Run:
```bash
cargo test -p codex-plus-core chat_completion_response_defaults_missing_reasoning_tokens_to_zero chat_sse_defaults_missing_reasoning_tokens_to_zero chat_sse_without_any_usage_still_emits_reasoning_tokens
```
Expected: the new tests fail because the generated usage object omits `reasoning_tokens`.

- [ ] **Step 3: Implement usage normalization**

Update `default_responses_usage` and `chat_usage_to_responses_usage` so every generated `output_tokens_details` object is present and has a numeric `reasoning_tokens` field, preserving upstream-provided values.

- [ ] **Step 4: Write failing Kimi reasoning tests**

Add a table-driven test for model `k3-256k` mapping `minimal/low -> low`, `medium/high -> high`, and `xhigh/max -> max`; assert `thinking.type == enabled`. Add an `effort: none` case that disables thinking and omits `reasoning_effort`.

- [ ] **Step 5: Run the Kimi test and verify failure**

Run:
```bash
cargo test -p codex-plus-core responses_request_maps_kimi_coding_reasoning_effort_per_official_spec
```
Expected: the test fails because K3 is classified as the default dialect and no Kimi effort field is emitted.

- [ ] **Step 6: Implement Kimi dialect mapping**

Recognize `k3*` and `*for-coding` IDs as Thinking models, add the Kimi-specific effort mapping, and emit `reasoning_effort` only for those models so existing GLM/MiMo/Kimi-thinking providers retain their current wire format.

- [ ] **Step 7: Run the focused protocol suite**

Run:
```bash
cargo test -p codex-plus-core protocol_proxy
```
Expected: all protocol proxy tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/codex-plus-core/src/protocol_proxy.rs crates/codex-plus-core/tests/protocol_proxy.rs
git commit -m "fix: harden proxy usage and Kimi reasoning mapping"
```

### Task 2: Native System Certificate Roots

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock` via Cargo resolution

**Interfaces:**
- Keep `http_client::proxied_client` and `vlm_http_client` APIs unchanged.

- [ ] **Step 1: Add the reqwest feature**

Add `rustls-tls-native-roots` beside the existing `rustls-tls` feature in the workspace reqwest dependency. Do not change default features or user-agent behavior.

- [ ] **Step 2: Resolve and compile**

Run:
```bash
cargo check -p codex-plus-core -p codex-plus-data
```
Expected: dependency resolution succeeds and both crates compile.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "fix: trust native certificate roots for relay requests"
```

### Task 3: DeepSeek Metadata and Model Catalog Builder

**Files:**
- Create: `assets/deepseek-model-metadata.json`
- Modify: `crates/codex-plus-core/src/model_suffix.rs`
- Modify: `crates/codex-plus-core/tests/model_suffix.rs`

**Interfaces:**
- Keep `build_model_catalog_json` and `build_model_catalog_json_with_template` signatures unchanged for existing callers.
- Add an internal metadata-aware builder parameter or helper without changing public model-window APIs.

- [ ] **Step 1: Write failing metadata tests**

Add tests that build catalogs for `deepseek-v4-flash` and `deepseek-v4-pro` and assert their metadata includes 1,048,576 context windows, high/max reasoning levels, and the expected `supported_in_api` values. Add a test that `model_ui_metadata("deepseek-v4-pro")` returns metadata.

- [ ] **Step 2: Run metadata tests and verify failure**

Run:
```bash
cargo test -p codex-plus-core --test model_suffix deepseek
```
Expected: tests fail because only GPT-5.6 metadata is bundled.

- [ ] **Step 3: Add the DeepSeek metadata asset**

Add the upstream-compatible entries for `deepseek-v4-flash` and `deepseek-v4-pro`, including context, reasoning, tool, visibility, and service-tier fields.

- [ ] **Step 4: Generalize metadata lookup**

Add a bundled DeepSeek JSON constant and a shared `catalog_metadata_entry` helper. Reuse it for GPT-5.6 lookup and add a DeepSeek-specific template merge path that starts from the bundled Codex template and overlays the metadata entry.

- [ ] **Step 5: Preserve window precedence**

Ensure the builder still chooses `entry.suffix_window`, then the supplied fallback window, then metadata `context_window`, then the existing 272000 default. Do not overwrite DeepSeek metadata fields such as `effective_context_window_percent` or `supported_in_api` when metadata is active.

- [ ] **Step 6: Run the model suffix suite**

Run:
```bash
cargo test -p codex-plus-core --test model_suffix
```
Expected: all existing and new metadata tests pass.

- [ ] **Step 7: Commit**

```bash
git add assets/deepseek-model-metadata.json crates/codex-plus-core/src/model_suffix.rs crates/codex-plus-core/tests/model_suffix.rs
git commit -m "feat: add DeepSeek model catalog metadata"
```

### Task 4: Relay Catalog Activation

**Files:**
- Modify: `crates/codex-plus-core/src/relay_config.rs`
- Modify: `crates/codex-plus-core/tests/relay_config.rs`

**Interfaces:**
- Keep `apply_model_catalog_to_config` behavior for user-supplied external catalogs unchanged.
- Extend `requires_bundled_metadata_catalog` or equivalent internal predicate to cover DeepSeek entries.

- [ ] **Step 1: Write failing relay-config tests**

Add a profile test with `id = "deepseek-test"`, `model_list = "deepseek-v4-pro"`, and no explicit window. Assert the generated config points to `model-catalogs/deepseek-test.json` and the file contains DeepSeek metadata. Add a second test with an explicit external `model_catalog_json` pointer and assert it remains unchanged.

- [ ] **Step 2: Run the relay-config tests and verify failure**

Run:
```bash
cargo test -p codex-plus-core --test relay_config deepseek
```
Expected: the metadata-only profile does not generate a managed catalog before the implementation change.

- [ ] **Step 3: Implement activation and capability preservation**

Recognize DeepSeek metadata entries as catalog-worthy, call the metadata-aware builder for them, and leave explicit external catalog pointers untouched. Keep per-model suffix windows and profile fallback windows authoritative over metadata defaults.

- [ ] **Step 4: Run focused relay tests**

Run:
```bash
cargo test -p codex-plus-core --test relay_config
```
Expected: all relay-config tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/codex-plus-core/src/relay_config.rs crates/codex-plus-core/tests/relay_config.rs
git commit -m "fix: activate DeepSeek metadata catalogs"
```

### Task 5: Full Verification

**Files:**
- No additional source files.

- [ ] **Step 1: Run formatting and diff checks**

Run:
```bash
cargo fmt --all -- --check
git diff --check
```
Expected: both commands exit successfully.

- [ ] **Step 2: Run the full workspace tests**

Run:
```bash
cargo test --workspace
```
Expected: zero failures.

- [ ] **Step 3: Verify fork safety and status**

Run:
```bash
git diff --stat 657cd33..HEAD
git status --short --branch
```
Expected: only the intended P0 commits are present and the working tree has no uncommitted changes.

- [ ] **Step 4: Commit any formatting-only adjustment if needed**

Only if `cargo fmt` changed tracked source after the prior task commits:
```bash
git add -u
git commit -m "chore: format P0 backport"
```

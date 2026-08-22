# Upstream P0 Backport Design

## Goal

Backport a small, Linux-safe subset of upstream CodexPlusPlus improvements into
`codex/linux-full-stack` without rebasing the fork or changing its Linux
packaging, launcher, project-state migration, or computer-use guard behavior.

## Scope

1. Harden the Responses protocol proxy.
   - Always emit `usage.output_tokens_details.reasoning_tokens` with a numeric
     zero fallback when an upstream omits the field or the whole details object.
   - Recognize Kimi For Coding model IDs (`k3*` and `*for-coding`) and map Codex
     reasoning efforts to Kimi's accepted `low`, `high`, and `max` values.
2. Enable native system certificate roots for reqwest clients.
3. Extend model catalog metadata for DeepSeek V4 models.
   - Add the upstream-compatible DeepSeek metadata asset.
   - Generalize the existing GPT-5.6 metadata lookup so DeepSeek entries can
     supply display, reasoning, tool, and context capabilities.
   - Preserve the existing precedence: explicit per-model window, profile
     fallback window, then metadata/default window.
   - Generate metadata-backed catalogs only through the existing Codex++ managed
     catalog path; keep explicit external `model_catalog_json` untouched.

## Design

`protocol_proxy.rs` remains the single conversion boundary for Chat
Completions-to-Responses usage and reasoning fields. Tests will exercise both
complete JSON responses and SSE completion paths, plus all Kimi effort values.

`model_suffix.rs` will expose one internal metadata lookup helper over bundled
JSON documents. GPT-5.6 and DeepSeek metadata will use the same catalog-entry
merge path. The catalog builder will accept an optional metadata source flag,
without changing callers that use the generic template builder.

`relay_config.rs` will recognize DeepSeek metadata entries as catalog-worthy in
the same way as existing GPT-5.6 entries. User-provided catalog pointers and
external catalogs retain current behavior.

## Verification

1. Add focused failing tests before implementation.
2. Run the focused core tests for protocol proxy, model suffix, and relay config.
3. Run `cargo test --workspace` and confirm the existing Linux test suite stays
   green.
4. Confirm `git diff --check` and a clean working tree apart from the intended
   commit(s).

## Non-goals

- No upstream merge or rebase.
- No provider-sync lock recovery, Stepwise protocol expansion, Bridge
  generation, renderer optimization, or UI changes in this batch.
- No version number bump to `1.2.50`.

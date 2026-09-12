# Reasoning Effort Fix for Non-GPT Models

## Problem
Only GPT models could adjust reasoning effort (推理强度); Claude, Grok, and other third-party models had no reasoning effort controls available.

## Root Cause
The bug was in `assets/inject/renderer-inject.js` at line 5972 in the `applyCodexPlusModelMetadata` function:

```javascript
// OLD BROKEN CODE
function applyCodexPlusModelMetadata(descriptor, modelName) {
  const metadata = codexPlusModelMetadata(modelName);
  if (!descriptor || !metadata) return false;  // ← Bug: exits early when metadata is null
  // ... rest of code never reached for non-GPT models
}
```

The function returned early when `metadata` was null, preventing the fallback reasoning efforts from being applied. The `modelReasoningEfforts` helper function (line 5962) already had a proper fallback for unknown models:

```javascript
function modelReasoningEfforts(modelName) {
  const supported = codexPlusModelMetadata(modelName)?.supportedReasoningEfforts;
  if (Array.isArray(supported) && supported.length > 0) {
    return supported.map((entry) => ({ ...entry }));
  }
  // Fallback: ["low", "medium", "high", "xhigh"]
  return ["low", "medium", "high", "xhigh"].map((reasoningEffort) => 
    ({ reasoningEffort, description: `${reasoningEffort} effort` })
  );
}
```

But this fallback was never invoked because of the early return.

## Solution
Restructured `applyCodexPlusModelMetadata` to:
1. Apply metadata fields (displayName, description, defaultReasoningEffort) only when metadata exists
2. **Always** apply reasoning efforts, using the fallback when metadata is null

```javascript
// FIXED CODE
function applyCodexPlusModelMetadata(descriptor, modelName) {
  if (!descriptor) return false;
  const metadata = codexPlusModelMetadata(modelName);
  let changed = false;

  // Apply metadata fields when available
  if (metadata) {
    for (const key of ["displayName", "description", "defaultReasoningEffort"]) {
      if (typeof metadata[key] === "string" && metadata[key] && descriptor[key] !== metadata[key]) {
        descriptor[key] = metadata[key];
        changed = true;
      }
    }
  }

  // Always ensure reasoning efforts are present (uses fallback when metadata is null)
  const nextEfforts = modelReasoningEfforts(modelName);
  if (JSON.stringify(descriptor.supportedReasoningEfforts || []) !== JSON.stringify(nextEfforts)) {
    descriptor.supportedReasoningEfforts = nextEfforts;
    changed = true;
  }

  return changed;
}
```

## Backend Support
The Rust backend (`crates/codex-plus-core/src/model_suffix.rs`) already provides generic fallback metadata for unknown models:

- `model_ui_metadata()` function (line 167) returns generic metadata with reasoning efforts for any unknown model
- `generic_model_ui_metadata()` function (line 202) provides the fallback
- Test coverage: `model_ui_metadata_provides_generic_fallback_for_unknown_models` (tests/model_suffix.rs:241)

The generic template (`assets/generic-model-template.json`) defines:
```json
{
  "default_reasoning_level": "medium",
  "supported_reasoning_levels": [
    { "effort": "low", "description": "Fast responses with lighter reasoning" },
    { "effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks" },
    { "effort": "high", "description": "Greater reasoning depth for complex problems" },
    { "effort": "xhigh", "description": "Extra high reasoning depth for complex problems" }
  ]
}
```

## Result
After this fix:
- ✅ GPT models: continue to work with their specific metadata
- ✅ Claude models: now have reasoning effort controls (low/medium/high/xhigh)
- ✅ Grok models: now have reasoning effort controls
- ✅ All custom/unknown models: get the generic fallback reasoning efforts

## Testing
Verified with:
1. Rust unit tests: `cargo test --package codex-plus-core --test model_suffix` (all 19 tests pass)
2. JavaScript behavior test: `test_reasoning_effort_fix.js` (validates the fixed logic)
3. Build verification: Both binaries built successfully

## Files Changed
- `assets/inject/renderer-inject.js` - Fixed the early return bug
- `crates/codex-plus-core/src/model_suffix.rs` - Updated outdated comment
- `test_reasoning_effort_fix.js` - JavaScript test demonstrating the fix

## Date
2026-09-04

# Linux ChatGPT Package Layout Compatibility Design

Date: 2026-08-12

## Background

Arch Linux upgraded `openai-codex-desktop` from `26.721.41059-2` to
`26.803.81509-8`. The new package installs the desktop application at
`/usr/lib/chatgpt`, while Codex++ currently auto-detects only its legacy
`/usr/lib/codex-plus-plus/app` wrapper. The saved Codex application path is
empty, so Codex++ selects that stale wrapper; the wrapper then looks for the
removed `/usr/lib/openai-codex-desktop` directory and exits before CDP starts.

The Linux process watcher also recognizes `app.asar` only when its path
contains `/openai-codex-desktop/`. Without recognizing `/chatgpt/`, lifecycle
tracking would still fail after application discovery is corrected.

## Goal

Launch the current Arch `openai-codex-desktop` package directly through the
normal Codex++ CDP launch path while retaining compatibility with the two
older Linux layouts.

## Design

`find_linux_codex_app_default` will check these directories in order:

1. `/usr/lib/chatgpt` for the current package layout.
2. `/usr/lib/openai-codex-desktop` for the previous package layout.
3. `/usr/lib/codex-plus-plus/app` for the legacy injected wrapper.

The existing path normalization remains the source of truth for validating
each candidate. An explicit command-line path or valid saved path keeps its
existing precedence.

The Linux process watcher will retain the previous package's `app.asar`
recognition and additionally treat `/usr/lib/chatgpt/ChatGPT` as the current
package's main process. A command containing `--type=...` is a Chromium child
process and remains excluded, as do unrelated Electron applications.

## Error Handling

If none of the supported layouts contains a valid desktop executable, the
existing `Codex App directory not found` error remains unchanged. No system
symlink or package-managed file is modified.

## Testing And Acceptance

- A regression test proves that default-candidate order prefers the current
  ChatGPT layout while falling back to the previous and legacy layouts.
- A watcher regression proves that `/usr/lib/chatgpt/ChatGPT` is recognized
  and its renderer subprocess is ignored.
- Focused and full `codex-plus-core` tests pass.
- Release binaries build and replace the user-level Codex++ 1.2.42 install.
- A real launch resolves `/usr/lib/chatgpt`, starts its desktop process, opens
  CDP port 9229, and reports a healthy Codex++ backend.

## Scope

This fix does not modify provider settings, Codex credentials, sessions,
`app.asar`, Arch package ownership, or `/usr/lib` contents.

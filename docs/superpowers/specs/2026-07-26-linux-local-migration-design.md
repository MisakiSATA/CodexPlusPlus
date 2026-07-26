# Linux Local Migration Design

## Objective

Make Codex++ a reliable, repeatable, user-scoped application on the current
Arch Linux x86_64 workstation. The result must install, discover and launch the
local Codex desktop application, survive upgrades, preserve Codex state, and
uninstall its own integration without requiring root privileges.

This is the first phase of the Linux migration. Ubuntu, Fedora, `.deb`, and
`.rpm` delivery will reuse the interfaces introduced here, but are not release
requirements for this phase.

## Current State

The existing Linux work already provides a usable development build:

- Codex++ and its manager build and run on Arch Linux.
- The packaged Codex desktop application can be launched with CDP enabled.
- Linux project paths and Codex project state are preserved correctly.
- Initial Linux support exists for application discovery, process scanning,
  `.desktop` entry generation, port fallback, and manager WebKitGTK startup.

It is not yet a complete migration:

- Generated desktop entries point at whichever development binaries launched
  the installer instead of a stable installed location.
- No application icon is installed and desktop/MIME caches are not refreshed.
- Codex discovery recognizes only one AUR-style directory layout.
- The updater has no Linux asset selection or safe replacement strategy.
- Watcher autostart remains Windows-only.
- Linux compatibility and degraded features are not reported coherently.
- CI does not build or test Linux artifacts.

## Scope

### In Scope

- Arch Linux x86_64 as the required runtime and smoke-test environment.
- User-scoped installation under XDG directories with no `sudo`.
- Stable application, desktop-entry, icon, protocol, and autostart paths.
- Validated discovery of the local Codex Electron desktop application.
- Safe Linux process detection and termination of Codex main processes.
- Reliable debug-port fallback and launcher lifecycle behavior.
- Preservation of Codex's own project list and manual project selection.
- A portable Linux zip artifact suitable for local installation and updates.
- Checksum-verified, staged, atomic update activation with rollback.
- Linux diagnostics and explicit reporting of unsupported platform features.
- Linux CI for Rust tests, frontend tests, type checking, release build, and
  portable artifact verification.

### Out of Scope

- Root-owned installation under `/usr` or `/opt`.
- AUR publication or package-manager orchestration.
- `.deb`, `.rpm`, AppImage, Flatpak, or Snap artifacts.
- Native ARM64 Linux artifacts.
- Reimplementing features that depend on Windows-only system APIs.
- Modifying the official Codex `app.asar` or its installed files.

## Architecture

### Linux Platform Boundary

Linux-specific installation and runtime policy belongs in focused modules
rather than being duplicated across UI commands and shared code:

- `install/linux.rs`: XDG layout, desktop entries, icon/protocol registration,
  autostart entry, version activation, rollback, and owned-file removal.
- `app_paths.rs`: candidate collection and strict Codex desktop validation.
- `watcher.rs`: `/proc` process observation and signal-based lifecycle control.
- `update.rs`: platform asset selection and delegation to the Linux staged
  installer without embedding UI behavior.
- manager commands: expose structured results and diagnostics; do not construct
  paths or shell commands in the frontend.

Shared modules continue to own platform-neutral types and orchestration.

### Installed Layout

The default user installation is:

```text
~/.local/lib/codex-plus-plus/
  current -> versions/<version>
  versions/<version>/
    bin/codex-plus-plus
    bin/codex-plus-plus-manager
    share/icon.png
    install-manifest.json
~/.local/share/applications/
  codex-plus-plus.desktop
  codex-plus-plus-manager.desktop
~/.local/share/icons/hicolor/256x256/apps/
  codex-plus-plus.png
~/.config/autostart/
  codex-plus-plus-watcher.desktop
```

`current` is a relative symbolic link changed with an atomic rename. Desktop
entries and protocol registration always reference binaries through `current`,
so an update never requires rewriting launch paths.

All removal is manifest-driven. Codex++, its updater, and its uninstaller may
remove only paths listed in an install manifest carrying the expected
`managedBy` marker. Codex configuration, authentication, projects, sessions,
and user-created files are never installation-owned.

### Portable Artifact

The first Linux release artifact is
`CodexPlusPlus-<version>-linux-x64.zip`. It contains both binaries, the icon, an
install manifest, and a user installer entrypoint. The archive layout matches a
single version directory and does not depend on the build checkout.

The release workflow also publishes a SHA-256 digest. `latest.json` includes
the digest beside the Linux asset. Linux updates are rejected when the digest
is missing or does not match.

## Runtime Flows

### Installation

1. Resolve XDG locations from the environment, falling back to standard paths
   below the current user's home directory.
2. Validate the two source binaries and artifact manifest.
3. Copy into a new version directory using restrictive temporary files, then
   set executable permissions.
4. Write the icon and desktop/autostart entries atomically.
5. Create a temporary relative `current` link and rename it into place.
6. Best-effort refresh the desktop database, icon cache, and URL handler using
   available XDG utilities. Missing cache utilities are diagnostic warnings,
   not installation failures.
7. Run a post-install inspection and return structured success or the exact
   failed component.

### Codex Discovery

Discovery order is deterministic:

1. Explicit path saved by the user.
2. A currently running Electron main process whose command line identifies the
   Codex desktop `app.asar`.
3. Known Arch/AUR and generic user/system installation candidates.
4. Candidate desktop files whose `Exec` target resolves to a valid bundle.

A candidate is accepted only when its directory contains a launchable Electron
entrypoint and a Codex desktop `resources/app.asar` relationship. A plain
`codex` CLI binary is never sufficient. Rejected candidates include a reason in
diagnostics rather than silently falling through.

### Launch And Process Lifecycle

The launcher preserves the existing CDP and helper flow. Busy debug and helper
ports fall back to available loopback ports on Linux. Only the validated Codex
Electron main process is tracked; renderer and unrelated Electron processes are
excluded.

Termination sends `SIGTERM`, waits with a bounded condition-based loop, and
reports remaining PIDs on timeout. It does not use name-wide kill commands and
does not send `SIGKILL` automatically.

Codex app-state synchronization keeps the Codex project list as the sole data
source. Saved projects remain visible, while `active-workspace-roots` is not
restored by snapshots so startup remains in manual project-selection mode.

### Manager Display Backend

The current XWayland-compatible startup remains the default on the target
machine because it is the verified stable path for the GTK3 tray and WebKitGTK.
Existing `GDK_BACKEND` and `WEBKIT_DISABLE_COMPOSITING_MODE` values always take
precedence. Diagnostics show the requested session, effective backend, and
whether software compositing fallback is active.

Native Wayland is an explicit opt-in during this phase. It can become the
default only after tray, dialog, window restoration, and black-screen smoke
tests pass without fallback on a supported environment.

### Update And Rollback

1. Select only a `linux-x64.zip` asset on x86_64 Linux.
2. Download the archive and verify the SHA-256 digest before extraction.
3. Reject absolute paths, parent traversal, links, duplicate destinations, and
   files absent from the artifact manifest.
4. Extract into a new version staging directory and validate both binaries.
5. Rename staging to the final version directory.
6. Atomically replace `current`, retaining the previous target as rollback
   metadata.
7. Launch the new manager with `--show-update`, then allow the old manager to
   exit.
8. If post-activation inspection fails, restore the previous link and report a
   recoverable error.

Old versions are pruned only after a later successful launch. The active and
immediately previous versions are always retained.

### Uninstall

Entry-point removal deletes only managed desktop, icon, MIME, and autostart
files. Full application removal additionally removes manifest-owned version
directories and the `current` link. User data removal remains a separate,
explicit option and never includes `~/.codex`.

## Feature Compatibility

The manager reports each platform-sensitive feature as supported, degraded, or
unavailable. Linux must not expose a successful toggle for behavior that is a
no-op.

- Supported: provider switching, model configuration, session data, CDP UI
  injection, project handling, scripts, local helper, and desktop entry repair.
- Degraded: XWayland manager backend and environment-only proxy discovery.
- Unavailable in this phase: Windows computer-use guard internals, Windows
  registry cleanup, and Windows native cursor integration.

## Error Handling And Diagnostics

- Every install/update stage returns a component name, path, and actionable
  error without embedding credentials or complete configuration contents.
- External utilities are invoked with argument arrays, never through a shell.
- Missing optional XDG cache utilities produce warnings.
- Missing required runtime libraries, invalid Codex layouts, busy ports, and
  permission errors are surfaced in the manager's health view.
- Update and install operations write recovery metadata before activation.
- Diagnostic logs redact API keys, auth state, provider secrets, and update
  URLs containing credentials.

## Verification

### Automated Tests

- Unit tests for XDG path resolution, desktop-entry escaping, manifests,
  ownership checks, archive validation, checksum verification, and symlink
  activation/rollback using temporary directories.
- Discovery tests covering the current AUR layout, generic layouts, running
  process evidence, invalid Electron apps, and Codex CLI false positives.
- Watcher tests covering main/renderer separation, bounded termination, and
  stale process state.
- Updater tests covering Linux asset/architecture selection and all rejected
  archive path forms.
- Frontend tests ensuring unsupported Linux controls are disabled or labelled
  and diagnostics render the effective display backend.

### Local Smoke Test

On the current Arch Linux x86_64 workstation:

1. Install from a freshly built portable artifact into isolated temporary XDG
   roots, then into the real user-scoped location after inspection.
2. Launch the manager from its desktop entry.
3. Detect and launch the installed Codex desktop application.
4. Verify CDP connection, provider configuration, old project visibility, and
   manual project selection.
5. Verify restart, busy-port fallback, watcher autostart, and graceful stop.
6. Perform an update rehearsal between two local version directories and test
   forced rollback.
7. Remove entrypoints and verify that Codex state remains intact.

### CI Gate

An Ubuntu Linux job runs frontend tests, TypeScript checking, Rust workspace
tests, a release build, portable artifact creation, archive inspection, and an
isolated XDG install/uninstall smoke test. The known unrelated dream-skin CDP
assertion must be fixed or explicitly isolated before Linux migration can be
called fully green.

## Completion Criteria

The local migration phase is complete when a clean portable artifact can be
installed without root, both applications launch from the desktop menu, Codex
projects and provider state survive restart/update, rollback is demonstrated,
uninstall preserves Codex data, Linux CI covers the artifact, and all scoped
tests pass. A successful development build alone is not completion.

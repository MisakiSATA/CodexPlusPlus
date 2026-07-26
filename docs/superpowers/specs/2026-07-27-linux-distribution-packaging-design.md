# Linux Distribution Packaging Design (Phase 2)

## Objective

Deliver Codex++ as native system packages — `.deb` (Debian/Ubuntu), `.rpm`
(Fedora/openSUSE), and an Arch `PKGBUILD` — reusing the runtime interfaces
established by the phase-1 user-scoped migration. Phase 1's portable zip and
user-scoped self-update remain the primary channel; system packages are an
additional distribution path whose files are owned by the system package
manager.

This fork deploys on Linux only. Windows and macOS packaging stay removed.

## Scope

### In Scope

- A shared system installation layout used by all three package formats.
- `scripts/installer/linux/package-deb.sh` producing a `.deb` with `dpkg-deb`.
- `scripts/installer/linux/package-rpm.sh` producing a `.rpm` with `rpmbuild`.
- `packaging/arch/PKGBUILD` building a binary Arch package from the released
  portable zip.
- Disabling in-app self-update when Codex++ runs from a system prefix, with a
  clear message pointing at the system package manager.
- CI: build and publish `.deb`/`.rpm` beside the portable zip on release, with
  SHA-256 digests.
- Local verification on the Arch workstation: `makepkg` build plus content
  inspection.

### Out of Scope

- AUR publication (the PKGBUILD is provided in-repo only).
- AppImage, Flatpak, Snap.
- ARM64 artifacts.
- Post-install system smoke tests inside CI containers.

## System Layout

All three formats install identical paths:

```text
/usr/bin/codex-plus-plus
/usr/bin/codex-plus-plus-manager
/usr/share/applications/codex-plus-plus.desktop
/usr/share/applications/codex-plus-plus-manager.desktop
/usr/share/icons/hicolor/256x256/apps/codex-plus-plus.png
```

Desktop entries reference `/usr/bin` directly. There is no `current` symlink
and no install manifest: version switching, upgrade, and removal belong to the
package manager. Runtime state stays in the user's `~/.codex-session-delete/`
and is never touched by packages.

Runtime dependencies per format:

- deb: `libwebkit2gtk-4.1-0`, `libgtk-3-0t64 | libgtk-3-0`,
  `libayatana-appindicator3-1`
- rpm: `webkit2gtk4.1`, `gtk3`, `libayatana-appindicator-gtk3`
- Arch: `webkit2gtk-4.1`, `gtk3`, `libayatana-appindicator`

`.deb` maintainer scripts and `.rpm` scriptlets refresh the desktop database
and icon cache best-effort. Arch relies on pacman hooks, which handle both
automatically.

## Self-Update Gating

A Codex++ binary running from `/usr/…` or `/opt/…` is owned by a system
package manager. In that case the Linux self-update path refuses to run and
tells the user to update through the package manager instead. The user-scoped
install under `~/.local/lib/codex-plus-plus/` keeps the phase-1 self-update
with digest verification and rollback.

The check is a pure path predicate in `install/linux.rs`, unit-tested, and
consulted by `update.rs` before any download is installed.

## CI

The release workflow gains a job that stages the shared system layout once and
produces both `.deb` and `.rpm` on `ubuntu-latest` (`dpkg-deb` is preinstalled;
`rpm` is installed via apt). Both artifacts and their `.sha256` files are
uploaded to the release. `latest.json` continues to list every asset with its
digest; the Linux updater still selects only the portable zip, so system
packages never enter the self-update path.

The PKGBUILD is not built in CI (no `makepkg` on Ubuntu runners); it is
validated locally on the Arch workstation.

## Verification

- Unit tests for the system-prefix predicate and the gated update error.
- `bash -n` syntax checks for both packaging scripts.
- Local Arch build: `makepkg` from the released zip, then `bsdtar` content
  inspection asserting the exact file list and permissions.
- CI builds `.deb`/`.rpm` on every release; `dpkg-deb --info/--contents` and
  `rpm -qpl` inspection steps assert the layout before upload.

## Completion Criteria

Phase 2 is complete when a release publishes portable zip, `.deb`, and `.rpm`
with digests, the in-repo PKGBUILD builds a valid Arch package locally, a
system-installed Codex++ refuses in-app self-update with an actionable
message, and all workspace tests stay green.

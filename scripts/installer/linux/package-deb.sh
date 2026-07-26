#!/usr/bin/env bash
# 构建 Debian/Ubuntu 包：codex-plus-plus_<version>_amd64.deb
# 用法: package-deb.sh <version>；环境 BINARY_DIR（默认 target/release）
# 需要 dpkg-deb（Ubuntu CI 自带）。
set -euo pipefail

VERSION="${1:?用法: package-deb.sh <version>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST_DIR="$REPO_ROOT/dist/linux"
STAGE_ROOT="$DIST_DIR/deb-root"
# Debian 版本号里的连字符表示修订分隔，预发布串统一换成 ~。
DEB_VERSION="${VERSION//-/\~}"
PACKAGE_NAME="codex-plus-plus"
ASSET_NAME="${PACKAGE_NAME}_${DEB_VERSION}_amd64.deb"

rm -rf "$STAGE_ROOT"
bash "$REPO_ROOT/scripts/installer/linux/stage-system-layout.sh" "$STAGE_ROOT"

mkdir -p "$STAGE_ROOT/DEBIAN"
INSTALLED_SIZE_KB="$(du -sk --exclude=DEBIAN "$STAGE_ROOT" | cut -f1)"
cat > "$STAGE_ROOT/DEBIAN/control" <<CONTROL
Package: $PACKAGE_NAME
Version: $DEB_VERSION
Section: devel
Priority: optional
Architecture: amd64
Installed-Size: $INSTALLED_SIZE_KB
Depends: libwebkit2gtk-4.1-0, libgtk-3-0t64 | libgtk-3-0, libayatana-appindicator3-1
Maintainer: MisakiSATA <codex-plus-plus@users.noreply.github.com>
Homepage: https://github.com/MisakiSATA/CodexPlusPlus
Description: Codex desktop enhancements and provider manager
 Codex++ launches the OpenAI Codex desktop application with CDP-based
 UI enhancements and manages providers, models and sessions.
 System-managed installs update through apt; in-app self-update is
 disabled for this package.
CONTROL

cat > "$STAGE_ROOT/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
update-desktop-database /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
exit 0
POSTINST

cat > "$STAGE_ROOT/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
update-desktop-database /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
exit 0
POSTRM

chmod 0755 "$STAGE_ROOT/DEBIAN/postinst" "$STAGE_ROOT/DEBIAN/postrm"

mkdir -p "$DIST_DIR"
rm -f "$DIST_DIR/$ASSET_NAME"
dpkg-deb --build --root-owner-group "$STAGE_ROOT" "$DIST_DIR/$ASSET_NAME"
(cd "$DIST_DIR" && sha256sum "$ASSET_NAME" > "$ASSET_NAME.sha256")

echo "打包完成: $DIST_DIR/$ASSET_NAME"
cat "$DIST_DIR/$ASSET_NAME.sha256"

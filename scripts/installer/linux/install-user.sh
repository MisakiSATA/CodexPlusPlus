#!/usr/bin/env bash
# Codex++ Linux 用户级安装入口。
# 在解压后的便携包目录内运行；全程只写当前用户的 XDG 目录，绝不使用 sudo。
# 目录布局与 crates/codex-plus-core/src/install/linux.rs 保持一致：
#   ~/.local/lib/codex-plus-plus/versions/<version>/  程序本体
#   ~/.local/lib/codex-plus-plus/current              原子切换的符号链接
#   ~/.local/share/applications/                      桌面入口
#   ~/.local/share/icons/hicolor/256x256/apps/        图标
set -euo pipefail

SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
LIB_ROOT="$HOME/.local/lib/codex-plus-plus"
APPLICATIONS_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/256x256/apps"

if [ ! -f "$SOURCE_DIR/install-manifest.json" ] \
  || [ ! -f "$SOURCE_DIR/bin/codex-plus-plus" ] \
  || [ ! -f "$SOURCE_DIR/bin/codex-plus-plus-manager" ]; then
  echo "请在解压后的 Codex++ 便携包目录内运行本脚本。" >&2
  exit 1
fi

VERSION="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$SOURCE_DIR/install-manifest.json" | head -1)"
if [ -z "$VERSION" ]; then
  echo "install-manifest.json 缺少版本号。" >&2
  exit 1
fi

VERSION_DIR="$LIB_ROOT/versions/$VERSION"
STAGING_DIR="$LIB_ROOT/versions/.staging-$VERSION-$$"

echo "安装 Codex++ $VERSION 到 $VERSION_DIR"
mkdir -p "$LIB_ROOT/versions"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR/bin" "$STAGING_DIR/share"
install -m 0755 "$SOURCE_DIR/bin/codex-plus-plus" "$STAGING_DIR/bin/codex-plus-plus"
install -m 0755 "$SOURCE_DIR/bin/codex-plus-plus-manager" "$STAGING_DIR/bin/codex-plus-plus-manager"
install -m 0644 "$SOURCE_DIR/share/icon.png" "$STAGING_DIR/share/icon.png"
install -m 0644 "$SOURCE_DIR/install-manifest.json" "$STAGING_DIR/install-manifest.json"
if [ -d "$VERSION_DIR" ]; then
  rm -rf "$VERSION_DIR"
fi
mv "$STAGING_DIR" "$VERSION_DIR"

# 原子切换 current 符号链接。
ln -sfn "versions/$VERSION" "$LIB_ROOT/.current-$$"
mv -T "$LIB_ROOT/.current-$$" "$LIB_ROOT/current"

LAUNCHER="$LIB_ROOT/current/bin/codex-plus-plus"
MANAGER="$LIB_ROOT/current/bin/codex-plus-plus-manager"

mkdir -p "$APPLICATIONS_DIR" "$ICON_DIR"
install -m 0644 "$SOURCE_DIR/share/icon.png" "$ICON_DIR/codex-plus-plus.png"

cat > "$APPLICATIONS_DIR/codex-plus-plus.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Codex++
Comment=Launch OpenAI Codex with Codex++ enhancements
Exec="$LAUNCHER" %U
Icon=codex-plus-plus
Terminal=false
Categories=Development;
StartupNotify=true
MimeType=x-scheme-handler/codexplusplus;
ENTRY

cat > "$APPLICATIONS_DIR/codex-plus-plus-manager.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Codex++ 管理工具
Comment=Manage Codex++ providers, models and enhancements
Exec="$MANAGER"
Icon=codex-plus-plus
Terminal=false
Categories=Development;
StartupNotify=true
ENTRY

# 缓存刷新是尽力而为：工具缺失只影响菜单刷新速度，不算安装失败。
update-desktop-database "$APPLICATIONS_DIR" 2>/dev/null || true
gtk-update-icon-cache -f -t "$DATA_HOME/icons/hicolor" 2>/dev/null || true
xdg-mime default codex-plus-plus.desktop x-scheme-handler/codexplusplus 2>/dev/null || true

echo "安装完成。可从应用菜单启动 Codex++ 与 Codex++ 管理工具，"
echo "或直接运行：$MANAGER"

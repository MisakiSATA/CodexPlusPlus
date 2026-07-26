#!/usr/bin/env bash
# 暂存 .deb/.rpm 共用的系统安装布局（/usr 前缀）。
# 用法: stage-system-layout.sh <stage_root>
# 环境: BINARY_DIR（默认 target/release）
# 布局必须与 docs/superpowers/specs/2026-07-27-linux-distribution-packaging-design.md 一致。
set -euo pipefail

STAGE_ROOT="${1:?用法: stage-system-layout.sh <stage_root>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BINARY_DIR="${BINARY_DIR:-$REPO_ROOT/target/release}"

for binary in codex-plus-plus codex-plus-plus-manager; do
  if [ ! -f "$BINARY_DIR/$binary" ]; then
    echo "缺少二进制: $BINARY_DIR/$binary" >&2
    exit 1
  fi
done

mkdir -p \
  "$STAGE_ROOT/usr/bin" \
  "$STAGE_ROOT/usr/share/applications" \
  "$STAGE_ROOT/usr/share/icons/hicolor/256x256/apps"

install -m 0755 "$BINARY_DIR/codex-plus-plus" "$STAGE_ROOT/usr/bin/codex-plus-plus"
install -m 0755 "$BINARY_DIR/codex-plus-plus-manager" "$STAGE_ROOT/usr/bin/codex-plus-plus-manager"
install -m 0644 "$REPO_ROOT/assets/images/codex-plus-plus.png" \
  "$STAGE_ROOT/usr/share/icons/hicolor/256x256/apps/codex-plus-plus.png"

cat > "$STAGE_ROOT/usr/share/applications/codex-plus-plus.desktop" <<'ENTRY'
[Desktop Entry]
Type=Application
Name=Codex++
Comment=Launch OpenAI Codex with Codex++ enhancements
Exec=/usr/bin/codex-plus-plus %U
Icon=codex-plus-plus
Terminal=false
Categories=Development;
StartupNotify=true
MimeType=x-scheme-handler/codexplusplus;
ENTRY

cat > "$STAGE_ROOT/usr/share/applications/codex-plus-plus-manager.desktop" <<'ENTRY'
[Desktop Entry]
Type=Application
Name=Codex++ 管理工具
Comment=Manage Codex++ providers, models and enhancements
Exec=/usr/bin/codex-plus-plus-manager
Icon=codex-plus-plus
Terminal=false
Categories=Development;
StartupNotify=true
ENTRY

chmod 0644 "$STAGE_ROOT/usr/share/applications/"*.desktop
echo "系统布局已暂存: $STAGE_ROOT"

#!/usr/bin/env bash
# 打包 Linux 便携 zip：CodexPlusPlus-<version>-linux-<arch>.zip
# 布局与用户级安装的版本目录一致（bin/、share/、install-manifest.json），
# 并生成 .sha256 摘要文件供 latest.json 与更新器校验。
set -euo pipefail

VERSION="${1:?用法: package-portable.sh <version> [arch]}"
ARCH="${2:-x64}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BINARY_DIR="${BINARY_DIR:-$REPO_ROOT/target/release}"
DIST_DIR="$REPO_ROOT/dist/linux"
STAGE_DIR="$DIST_DIR/app-$ARCH"
ASSET_NAME="CodexPlusPlus-$VERSION-linux-$ARCH.zip"

for binary in codex-plus-plus codex-plus-plus-manager; do
  if [ ! -f "$BINARY_DIR/$binary" ]; then
    echo "缺少二进制: $BINARY_DIR/$binary" >&2
    exit 1
  fi
done

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/share"
install -m 0755 "$BINARY_DIR/codex-plus-plus" "$STAGE_DIR/bin/codex-plus-plus"
install -m 0755 "$BINARY_DIR/codex-plus-plus-manager" "$STAGE_DIR/bin/codex-plus-plus-manager"
install -m 0644 "$REPO_ROOT/assets/images/codex-plus-plus.png" "$STAGE_DIR/share/icon.png"
install -m 0755 "$REPO_ROOT/scripts/installer/linux/install-user.sh" "$STAGE_DIR/install.sh"

# managedBy 标记必须与 crates/codex-plus-core/src/install/linux.rs 的
# INSTALL_MANAGED_BY 保持一致，更新器只接受受管清单。
cat > "$STAGE_DIR/install-manifest.json" <<MANIFEST
{
  "managedBy": "Codex++ user install",
  "version": "$VERSION",
  "files": [
    "bin/codex-plus-plus",
    "bin/codex-plus-plus-manager",
    "share/icon.png",
    "install.sh"
  ]
}
MANIFEST

mkdir -p "$DIST_DIR"
rm -f "$DIST_DIR/$ASSET_NAME"
(cd "$STAGE_DIR" && zip -rq "$DIST_DIR/$ASSET_NAME" .)
(cd "$DIST_DIR" && sha256sum "$ASSET_NAME" > "$ASSET_NAME.sha256")

echo "打包完成: $DIST_DIR/$ASSET_NAME"
cat "$DIST_DIR/$ASSET_NAME.sha256"

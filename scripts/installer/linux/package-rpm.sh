#!/usr/bin/env bash
# 构建 RPM 包：codex-plus-plus-<version>-1.x86_64.rpm
# 用法: package-rpm.sh <version>；环境 BINARY_DIR（默认 target/release）
# 需要 rpmbuild（Ubuntu CI 上通过 apt 安装 rpm 包获得）。
set -euo pipefail

VERSION="${1:?用法: package-rpm.sh <version>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST_DIR="$REPO_ROOT/dist/linux"
STAGE_ROOT="$DIST_DIR/rpm-root"
TOP_DIR="$DIST_DIR/rpm-topdir"
# RPM 版本号不允许连字符，预发布串统一换成 ~。
RPM_VERSION="${VERSION//-/\~}"
PACKAGE_NAME="codex-plus-plus"

rm -rf "$STAGE_ROOT" "$TOP_DIR"
bash "$REPO_ROOT/scripts/installer/linux/stage-system-layout.sh" "$STAGE_ROOT"
mkdir -p "$TOP_DIR/SPECS" "$TOP_DIR/RPMS"

cat > "$TOP_DIR/SPECS/$PACKAGE_NAME.spec" <<SPEC
Name:           $PACKAGE_NAME
Version:        $RPM_VERSION
Release:        1
Summary:        Codex desktop enhancements and provider manager
License:        MIT
URL:            https://github.com/MisakiSATA/CodexPlusPlus
Requires:       webkit2gtk4.1, gtk3, libayatana-appindicator-gtk3
%define _stage $STAGE_ROOT
%define _build_id_links none
%global __strip /bin/true
%global debug_package %{nil}

%description
Codex++ launches the OpenAI Codex desktop application with CDP-based
UI enhancements and manages providers, models and sessions.
System-managed installs update through dnf/zypper; in-app self-update
is disabled for this package.

%install
mkdir -p %{buildroot}
cp -a %{_stage}/. %{buildroot}/

%post
update-desktop-database /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || :

%postun
update-desktop-database /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || :

%files
/usr/bin/codex-plus-plus
/usr/bin/codex-plus-plus-manager
/usr/share/applications/codex-plus-plus.desktop
/usr/share/applications/codex-plus-plus-manager.desktop
/usr/share/icons/hicolor/256x256/apps/codex-plus-plus.png
SPEC

rpmbuild -bb \
  --define "_topdir $TOP_DIR" \
  --target x86_64 \
  "$TOP_DIR/SPECS/$PACKAGE_NAME.spec"

RPM_FILE="$(find "$TOP_DIR/RPMS" -name "*.rpm" | head -1)"
if [ -z "$RPM_FILE" ]; then
  echo "rpmbuild 未产出 RPM 文件" >&2
  exit 1
fi
ASSET_NAME="$(basename "$RPM_FILE")"
mv "$RPM_FILE" "$DIST_DIR/$ASSET_NAME"
(cd "$DIST_DIR" && sha256sum "$ASSET_NAME" > "$ASSET_NAME.sha256")

echo "打包完成: $DIST_DIR/$ASSET_NAME"
cat "$DIST_DIR/$ASSET_NAME.sha256"

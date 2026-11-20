#!/usr/bin/env bash
set -euo pipefail
[[ $(id -un) == admin && ${ENZUI_DESKTOP_SESSION:-} == installed ]]
qa_work="${XDG_CACHE_HOME:-$HOME/.cache}/enzui-permissions-qa"
[[ ! -e "$qa_work" ]]
install -d -m700 "$qa_work"
export LC_ALL=C
printf 'ENZUI_PERMISSIONS_ENV\n'
env | sort | sed -n '/^HOME=/p;/^XDG_.*_HOME=/p;/^ENZUI_DESKTOP_SESSION=/p'
flatpak build-init --arch=x86_64 "$qa_work/build" org.enzui.PermissionsQA org.freedesktop.Platform org.freedesktop.Platform 25.08
install -d "$qa_work/build/files/bin" "$qa_work/build/files/share/applications" "$qa_work/build/files/share/metainfo" "$qa_work/build/files/share/icons/hicolor/scalable/apps"
install -m755 /run/enzui-permissions-qa/permissions-qa "$qa_work/build/files/bin/permissions-qa"
cat > "$qa_work/build/files/share/applications/org.enzui.PermissionsQA.desktop" <<'APP'
[Desktop Entry]
Type=Application
Name=enzUI Permissions QA
Comment=Public sandbox permission fixture
Exec=permissions-qa notify menu-default
Icon=org.enzui.PermissionsQA
Categories=Utility;
Terminal=false
APP
cat > "$qa_work/build/files/share/metainfo/org.enzui.PermissionsQA.metainfo.xml" <<'APP'
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application"><id>org.enzui.PermissionsQA</id><metadata_license>CC0-1.0</metadata_license><project_license>MIT</project_license><name>enzUI Permissions QA</name><summary>Public sandbox permission fixture</summary><description><p>Exercises notification and background consent through the desktop portal.</p></description><launchable type="desktop-id">org.enzui.PermissionsQA.desktop</launchable><releases><release version="1.0" date="2026-10-15"/></releases></component>
APP
cat > "$qa_work/build/files/share/icons/hicolor/scalable/apps/org.enzui.PermissionsQA.svg" <<'APP'
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect x="4" y="4" width="56" height="56" rx="12" fill="#336dcc"/><path d="M18 32l10 10 19-21" fill="none" stroke="white" stroke-width="6"/></svg>
APP
flatpak build-finish --no-inherit-permissions --command=permissions-qa --share=network --filesystem=xdg-documents:ro --socket=pulseaudio --metadata=X-PermissionsQA.OpaquePolicy=keep-me "$qa_work/build"
flatpak build-export "$qa_work/repo" "$qa_work/build" stable
flatpak build-update-repo "$qa_work/repo"
flatpak --user remote-add --no-gpg-verify enzui-permissions-qa "file://$qa_work/repo"
flatpak --user install --assumeyes --noninteractive --no-deps --no-related enzui-permissions-qa org.enzui.PermissionsQA
flatpak --user list --app --columns=application,branch,installation
printf "ENZUI_PERMISSIONS_FIXTURE_INSTALLED\n"

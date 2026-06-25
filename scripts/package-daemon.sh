#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SRC_TAURI_DIR="${REPO_ROOT}/src-tauri"

PROFILE="${PROFILE:-release}"
VERSION="${VERSION:-$(date +%Y%m%d%H%M%S)}"
TARGET_DIR="${SRC_TAURI_DIR}/target/${PROFILE}"
PACKAGE_ROOT="${REPO_ROOT}/dist"
PACKAGE_DIR="${PACKAGE_ROOT}/codex-monitor-daemon-${VERSION}-${PROFILE}"
ARCHIVE_PATH="${PACKAGE_DIR}.tar.gz"

echo "[package-daemon] profile: ${PROFILE}"
echo "[package-daemon] version: ${VERSION}"

"${REPO_ROOT}/scripts/build-daemon.sh"

rm -rf "${PACKAGE_DIR}"
mkdir -p "${PACKAGE_DIR}/bin"
mkdir -p "${PACKAGE_DIR}/deploy"
mkdir -p "${PACKAGE_DIR}/docs"

cp "${TARGET_DIR}/codex_monitor_daemon" "${PACKAGE_DIR}/bin/"
cp "${TARGET_DIR}/codex_monitor_daemonctl" "${PACKAGE_DIR}/bin/"
cp "${REPO_ROOT}/deploy/codex-monitor-daemon.service" "${PACKAGE_DIR}/deploy/"
cp "${REPO_ROOT}/deploy/nginx-codex-monitor-daemon.conf" "${PACKAGE_DIR}/deploy/"
cp "${REPO_ROOT}/docs/deploy-daemon.md" "${PACKAGE_DIR}/docs/"
cp "${REPO_ROOT}/REMOTE_BACKEND_POC.md" "${PACKAGE_DIR}/docs/"

cat > "${PACKAGE_DIR}/README.txt" <<'EOF'
Codex Monitor Daemon Package

Contents:
- bin/codex_monitor_daemon
- bin/codex_monitor_daemonctl
- deploy/codex-monitor-daemon.service
- deploy/nginx-codex-monitor-daemon.conf
- docs/deploy-daemon.md

Quick start:
1. Copy bin/ to /opt/codex-monitor-daemon/bin
2. Install deploy/codex-monitor-daemon.service to /etc/systemd/system/
3. Create /etc/codex-monitor-daemon/codex-monitor-daemon.env
4. Start with: systemctl enable --now codex-monitor-daemon
EOF

tar -C "${PACKAGE_ROOT}" -czf "${ARCHIVE_PATH}" "$(basename "${PACKAGE_DIR}")"

echo "[package-daemon] package dir: ${PACKAGE_DIR}"
echo "[package-daemon] archive: ${ARCHIVE_PATH}"

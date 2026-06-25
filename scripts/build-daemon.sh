#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SRC_TAURI_DIR="${REPO_ROOT}/src-tauri"

PROFILE="${PROFILE:-release}"
TARGET_DIR="${SRC_TAURI_DIR}/target/${PROFILE}"

case "${PROFILE}" in
  release)
    PROFILE_FLAG="--release"
    ;;
  debug)
    PROFILE_FLAG=""
    ;;
  *)
    echo "[build-daemon] unsupported PROFILE: ${PROFILE}" >&2
    echo "[build-daemon] use PROFILE=release or PROFILE=debug" >&2
    exit 1
    ;;
esac

echo "[build-daemon] repo root: ${REPO_ROOT}"
echo "[build-daemon] cargo profile: ${PROFILE}"

cd "${SRC_TAURI_DIR}"

cargo build \
  ${PROFILE_FLAG} \
  --no-default-features \
  --bin codex_monitor_daemon \
  --bin codex_monitor_daemonctl

echo "[build-daemon] built:"
echo "  ${TARGET_DIR}/codex_monitor_daemon"
echo "  ${TARGET_DIR}/codex_monitor_daemonctl"

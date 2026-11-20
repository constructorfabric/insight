#!/usr/bin/env sh
set -eu

if [ $# -lt 3 ] || [ "$2" != "--" ]; then
  echo "usage: docker-entrypoint.sh <watched-path> -- <command> [args...]" >&2
  exit 2
fi

WATCH_PATH="$1"
shift 2

case "${ENABLE_AUTO_RELOAD:-}" in
  true|1)
    if [ -d "$WATCH_PATH" ]; then
      WATCH_DIR="$WATCH_PATH"
    else
      WATCH_DIR="$(dirname "$WATCH_PATH")"
    fi
    echo "[entrypoint] ENABLE_AUTO_RELOAD=true -- watchexec on ${WATCH_DIR}"
    exec watchexec \
      --restart \
      --no-vcs-ignore \
      --no-project-ignore \
      --debounce 500ms \
      --watch "${WATCH_DIR}" \
      -- "$@"
    ;;
  *)
    echo "[entrypoint] ENABLE_AUTO_RELOAD unset -- exec direct"
    exec "$@"
    ;;
esac

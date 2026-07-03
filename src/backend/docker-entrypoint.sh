#!/usr/bin/env sh
# Shared entrypoint for the Insight backend service containers.
#
# Copied into the image by each service Dockerfile (api-gateway,
# analytics, identity). Behaviour switches on ENABLE_AUTO_RELOAD:
#
#   unset / anything else  : exec the command directly (prod default)
#   true | 1               : wrap the command in `watchexec` so that a
#                            change to the watched binary file triggers
#                            SIGTERM + respawn. Used by the local
#                            docker-compose dev stack with bind-mounted
#                            binaries.
#
# k8s policy: NEVER set ENABLE_AUTO_RELOAD in cluster manifests. The
# variable is opt-in by docker-compose only.
#
# Usage:  docker-entrypoint.sh <watched-path> -- <command> [args...]
set -eu

if [ $# -lt 3 ] || [ "$2" != "--" ]; then
  echo "usage: docker-entrypoint.sh <watched-path> -- <command> [args...]" >&2
  exit 2
fi

WATCH_PATH="$1"
shift 2

case "${ENABLE_AUTO_RELOAD:-}" in
  true|1)
    # watchexec 2.3+ wants a directory; if a file was passed (the
    # intuitive "restart when THIS binary changes"), watch its parent
    # instead. /app only contains the service's runtime files so this
    # doesn't false-positive on unrelated edits.
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

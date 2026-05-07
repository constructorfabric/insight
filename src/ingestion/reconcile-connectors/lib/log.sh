#!/usr/bin/env bash
# log.sh — file logger with quiet-run policy.
# Sourceable; NO top-level CLI.
#
# Public surface:
#   log_init                 # opens daily-rotated log file on fd 9
#   log_line LEVEL MSG ...   # writes one line if MSG non-empty; NOOP otherwise
#   log_run_summary CHANGES ERRS
#                            # ALWAYS emits one stdout line; flushes file IFF
#                            # CHANGES>0 OR ERRS>0
#   log_close                # closes fd 9
#
# Quiet-run policy (per KEY DECISION #6): on a no-op reconcile iteration,
# log_line is never called for the no-op branch, AND log_run_summary writes
# only stdout (one line). The log file size is unchanged.

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

LOG_TARGET=""
LOG_FD_OPEN=0

log_init() {
  # Resolve target via lib/env.sh:env_log_target_resolve.
  LOG_TARGET="$(env_log_target_resolve)"
  mkdir -p "$(dirname "$LOG_TARGET")"
  # fd 9 left closed until first log_line; we open lazily so quiet runs leave
  # mtime/size untouched.
}

# @cpt-begin:cpt-insightspec-algo-reconcile-write-log-line-on-change:p1
log_line() {
  local level="$1"; shift
  local msg="$*"
  if [[ -z "$msg" ]]; then
    return 0   # quiet-run safety: empty message → noop
  fi
  if [[ -z "${LOG_TARGET:-}" ]]; then
    log_init
  fi
  # Open fd 9 lazily on first non-empty write; cached for subsequent calls.
  if [[ "${LOG_FD_OPEN}" -eq 0 ]]; then
    if ! { exec 9>>"$LOG_TARGET"; } 2>/dev/null; then
      printf 'log_line: cannot open %s\n' "$LOG_TARGET" >&2
      return 1
    fi
    LOG_FD_OPEN=1
  fi
  printf '%s [%s] %s\n' "$(date -u +%FT%TZ)" "$level" "$msg" >&9
}
# @cpt-end:cpt-insightspec-algo-reconcile-write-log-line-on-change:p1

log_run_summary() {
  local changes="${1:-0}"
  local errs="${2:-0}"
  if [[ "$changes" -eq 0 && "$errs" -eq 0 ]]; then
    printf 'reconcile finished: nothing to do\n'
  else
    printf 'reconcile finished: %s change(s), %s error(s)  (log: %s)\n' \
      "$changes" "$errs" "${LOG_TARGET:-<uninitialised>}"  # RULE-DEFAULTS-OK: display-only label in summary line; not a config input
    log_line INFO "summary: $changes change(s), $errs error(s)" || true
  fi
}

log_close() {
  exec 9>&- 2>/dev/null || true
  LOG_TARGET=""
  LOG_FD_OPEN=0
}

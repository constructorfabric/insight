#!/usr/bin/env bash
#
# push-deploy-log.sh — ship a deploy log file into the in-cluster Loki so
# every umbrella deploy is visible in Grafana ({job="deploy-insight"}; see
# the "Insight · Ingestion & deploys" dashboard).
#
# Called by the Makefile's deploy-insight target after `helm upgrade`,
# ALWAYS best-effort: any failure here must never fail the deploy, so the
# script exits 0 unconditionally. Failed-deploy logs are shipped too —
# those are the ones you want to read later.
#
# Usage: push-deploy-log.sh <env> <logfile> <version>
# Env:   NS_INFRA  namespace of the Loki release (default insight-infra)
#        KUBE_CTX  kube context to use (default: current context)
#
set -uo pipefail

ENVNAME="${1:?usage: push-deploy-log.sh <env> <logfile> <version>}"
LOGFILE="${2:?usage: push-deploy-log.sh <env> <logfile> <version>}"
VERSION="${3:-unknown}"
NS_INFRA="${NS_INFRA:-insight-infra}"
CTX_ARGS=()
[ -n "${KUBE_CTX:-}" ] && CTX_ARGS=(--context "$KUBE_CTX")

note() { printf 'push-deploy-log: %s\n' "$*"; }

command -v jq >/dev/null 2>&1 || { note "jq not found — skipping"; exit 0; }
[ -s "$LOGFILE" ] || { note "log file empty or missing — skipping"; exit 0; }

# Loki is ClusterIP-only; tunnel for the duration of the push.
kubectl "${CTX_ARGS[@]}" -n "$NS_INFRA" port-forward svc/loki 3100:3100 >/dev/null 2>&1 &
PF=$!
trap 'kill "$PF" 2>/dev/null' EXIT

READY=0
for _ in $(seq 1 20); do
  if curl -sf http://localhost:3100/ready >/dev/null 2>&1; then READY=1; break; fi
  sleep 0.5
done
[ "$READY" = "1" ] || { note "Loki not reachable — skipping"; exit 0; }

# One Loki stream per deploy; per-line ns timestamps = epoch seconds of the
# push + the line number as the nanosecond part, so ordering is preserved
# and no two entries collide.
TS="$(date +%s)"
if jq -Rn --arg env "$ENVNAME" --arg ver "$VERSION" --arg ts "$TS" '
     {streams: [{
       stream: {job: "deploy-insight", env: $env, version: $ver},
       values: [inputs | [($ts + ((1000000000 + input_line_number) | tostring | .[1:10])), .]]
     }]}' < "$LOGFILE" \
   | curl -sf -XPOST -H 'Content-Type: application/json' --data-binary @- \
       http://localhost:3100/loki/api/v1/push; then
  note "shipped $(wc -l < "$LOGFILE" | tr -d ' ') lines as {job=\"deploy-insight\", env=\"$ENVNAME\", version=\"$VERSION\"}"
else
  note "push failed (non-fatal)"
fi
exit 0

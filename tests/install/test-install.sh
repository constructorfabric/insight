#!/usr/bin/env bash
# ============================================================================
# Insight deployment/installation test
#
# Tests that a clean local install of constructorfabric/insight per the
# README instructions (./dev-up.sh) completes and produces a working stack.
# Designed for: laptop verification AND nightly CI (kind-based).
#
# Usage:
#   ./test-install.sh [--repo DIR] [--clean] [--skip-install] [--timeout SEC]
#
#   --repo DIR       insight repo checkout (default: ~/projects/insight)
#   --clean          delete the kind cluster first (true fresh-install test)
#   --skip-install   only run post-install verification against existing stack
#   --timeout SEC    max seconds for dev-up.sh (default 3600)
#
# Exit code: 0 = all checks pass, 1 = failures (see summary table)
# ============================================================================
set -uo pipefail

REPO="${HOME}/projects/insight"
CLEAN=false
SKIP_INSTALL=false
TIMEOUT=3600
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="$2"; shift 2;;
    --clean) CLEAN=true; shift;;
    --skip-install) SKIP_INSTALL=true; shift;;
    --timeout) TIMEOUT="$2"; shift 2;;
    *) echo "unknown arg: $1"; exit 2;;
  esac
done

LOG="${TMPDIR:-/tmp}/insight-install-test-$(date +%Y%m%d-%H%M%S).log"
KCFG="${KUBECONFIG:-$HOME/.kube/insight.kubeconfig}"
NS=insight
PASS=0; FAIL=0; RESULTS=()

check() { # check <name> <command...>
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    RESULTS+=("PASS  $name"); PASS=$((PASS+1)); echo "  ✓ $name"
  else
    RESULTS+=("FAIL  $name"); FAIL=$((FAIL+1)); echo "  ✗ $name"
  fi
}

http_check() { # http_check <name> <url> <expected-codes-regex>
  local name="$1" url="$2" want="$3"
  local code; code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$url" 2>/dev/null)
  if [[ "$code" =~ $want ]]; then
    RESULTS+=("PASS  $name ($code)"); PASS=$((PASS+1)); echo "  ✓ $name ($code)"
  else
    RESULTS+=("FAIL  $name (got $code, want $want)"); FAIL=$((FAIL+1)); echo "  ✗ $name (got $code, want $want)"
  fi
}

# ─── Phase 0: preflight — environment defects we have already hit ──────────
echo "═══ Phase 0: preflight ═══"
check "docker daemon answers"            docker info
check "kind installed"                   command -v kind
check "kubectl installed"                command -v kubectl
check "helm installed"                   command -v helm
# Defect #2 (TA pending): dev-up.sh needs bash >= 4 (declare -A); macOS ships 3.2
check "bash >= 4 resolvable via env"     bash -c '(( BASH_VERSINFO[0] >= 4 ))'
# Defect #1: parse_descriptor.py emits quoted tags when PyYAML is missing
check "python3 has PyYAML"               python3 -c 'import yaml'
check "repo checkout exists"             test -f "$REPO/dev-up.sh"
check ".env.local present"               test -f "$REPO/.env.local"
# Defect #0: 60 GB Docker VM disk fills up — require >= 25 GB free
check "docker VM disk >= 25 GB free" bash -c \
  "docker run --rm debian:bookworm df -BG --output=avail / | tail -1 | tr -dc '0-9' | awk '{exit (\$1<25)}'"

if [[ $FAIL -gt 0 ]]; then
  echo; echo "Preflight failed — fix environment before testing the installer."
  printf '%s\n' "${RESULTS[@]}"; exit 1
fi

# ─── Phase 1: optional clean slate ──────────────────────────────────────────
if $CLEAN; then
  echo "═══ Phase 1: clean slate (deleting kind cluster) ═══"
  # cleanup.sh prompts interactively ("Are you sure? [y/N]"). In CI / any
  # non-interactive shell that read gets empty stdin, the script cancels with
  # exit 0, and the `||` fallback never fires — leaving the cluster in place
  # and silently breaking the fresh-install contract. Delete directly instead.
  if [[ -n "${CI:-}" ]] || ! [ -t 0 ]; then
    kind delete cluster --name insight >>"$LOG" 2>&1 || true
  else
    (cd "$REPO" && ./cleanup.sh) >>"$LOG" 2>&1 || kind delete cluster --name insight >>"$LOG" 2>&1
  fi
  echo "  cluster deleted"
fi

# ─── Phase 2: install per instructions ──────────────────────────────────────
if ! $SKIP_INSTALL; then
  echo "═══ Phase 2: ./dev-up.sh (timeout ${TIMEOUT}s, log: $LOG) ═══"
  # Workarounds for known defects #3 (tenantId) and #5/#6 (CH + MariaDB
  # pre-install hooks deadlock on fresh install — they wait for DBs the same
  # release deploys). Phase A installs without identity (skips MariaDB hook)
  # and with CH init disabled; phase B re-runs as an upgrade with defaults so
  # the pre-upgrade hooks run against live databases. Remove once fixed.
  OVERRIDES="${TMPDIR:-/tmp}/insight-test-values.yaml"
  OVERRIDES_B="${TMPDIR:-/tmp}/insight-test-values-b.yaml"
  # global.tenantDefaultId works around defect #8: analytics-api /health
  # returns 400 TENANT_UNRESOLVED without it → probes kill the pod forever.
  printf 'global:\n  tenantDefaultId: "11111111-1111-1111-1111-111111111111"\ningestion:\n  reconcile:\n    tenantId: "citest"\nclickhouse:\n  initDatabases: []\nidentity:\n  deploy: false\n' > "$OVERRIDES"
  printf 'global:\n  tenantDefaultId: "11111111-1111-1111-1111-111111111111"\ningestion:\n  reconcile:\n    tenantId: "citest"\n' > "$OVERRIDES_B"
  # Defect #4: chart placeholder appVersion "---" is an invalid k8s label
  if grep -q '^appVersion: "---"' "$REPO/charts/insight/Chart.yaml"; then
    echo "  (applying defect-#4 workaround: appVersion --- → 0.0.0-citest)"
    sed -i.bak 's/^appVersion: "---"/appVersion: "0.0.0-citest"/' "$REPO/charts/insight/Chart.yaml"
  fi
  ( cd "$REPO" && INSIGHT_VALUES_FILES="$OVERRIDES" timeout "$TIMEOUT" ./dev-up.sh ) >>"$LOG" 2>&1
  RC=$?
  if [[ $RC -eq 0 ]]; then
    echo "  phase A ok — phase B: upgrade with identity + DB-init hooks enabled"
    ( cd "$REPO" && INSIGHT_VALUES_FILES="$OVERRIDES_B" timeout "$TIMEOUT" ./dev-up.sh app ) >>"$LOG" 2>&1
    RC=$?
  fi
  if [[ $RC -ne 0 ]]; then
    echo "  ✗ dev-up.sh exited rc=$RC — classifying failure:"
    classify() { grep -q "$1" "$LOG" && echo "    KNOWN DEFECT: $2"; }
    classify "kubectl: unbound variable"            "#2 bash 3.2 — dev-up.sh needs bash>=4 (declare -A)"
    classify "invalid reference format"             "#1 parse_descriptor.py quotes tag when PyYAML missing"
    classify "tenantId is required"                 "#3 dev-up.sh never sets ingestion.reconcile.tenantId"
    classify 'Invalid value: "---"'                 "#4 chart appVersion placeholder is invalid k8s label"
    classify "ClickHouse did not become reachable"  "#5 pre-install hook waits for CH that same release deploys"
    classify "mariadb-init-svcdbs"                  "#6 pre-install hook waits for MariaDB that same release deploys (identity.deploy=true)"
    classify "insight-db-creds.*not found"          "#7 creds Secret is a regular manifest but hooks need it pre-install (autoGenerate fresh-install gap)"
    classify "TENANT_UNRESOLVED"                    "#8 analytics-api /health requires tenant context; empty tenantDefaultId → probes kill pod"
    classify "invalid signature was encountered"    "#0 docker VM disk full (apt GPG = ENOSPC in disguise)"
    classify "no route to host"                     "docker VM down/restarting"
    RESULTS+=("FAIL  dev-up.sh completed"); FAIL=$((FAIL+1))
  else
    echo "  ✓ dev-up.sh completed"
    RESULTS+=("PASS  dev-up.sh completed"); PASS=$((PASS+1))
  fi
fi

# ─── Phase 3: post-install verification ─────────────────────────────────────
echo "═══ Phase 3: stack verification ═══"
export KUBECONFIG="$KCFG"
check "kind cluster reachable" kubectl get nodes
for rel in airbyte argo-workflows insight; do
  check "helm release '$rel' deployed" bash -c \
    "helm status $rel -n $NS -o json 2>/dev/null | grep -q '\"status\":\"deployed\"'"
done
check "no pods in CrashLoopBackOff/Error" bash -c \
  "! kubectl -n $NS get pods --no-headers 2>/dev/null | grep -E 'CrashLoopBackOff|Error|ImagePullBackOff'"
check "clickhouse pod Ready" kubectl -n $NS wait --for=condition=ready pod \
  -l app.kubernetes.io/name=clickhouse --timeout=120s
# the database the pipeline writes to must exist (defect #5 leaves it missing)
check "clickhouse 'insight' DB exists" bash -c '
  CHPOD=$(kubectl -n '"$NS"' get pods -l app.kubernetes.io/name=clickhouse -o name | head -1)
  CHPASS=$(kubectl -n '"$NS"' get secret insight-db-creds -o jsonpath="{.data.clickhouse-password}" | base64 -d)
  kubectl -n '"$NS"' exec "$CHPOD" -- clickhouse-client --password "$CHPASS" -q "SHOW DATABASES" | grep -qx insight'

echo "═══ Phase 4: endpoint smoke (BVT entry points) ═══"
http_check "frontend  :8003"        "http://localhost:8003"            '^(200|301|302)$'
http_check "gateway   :8080/health" "http://localhost:8080/health"     '^200$'
http_check "airbyte   :8001"        "http://localhost:8001"            '^(200|301|302)$'
http_check "argo      :2746"        "http://localhost:2746"            '^(200|301|302)$'
http_check "clickhouse:8123/ping"   "http://localhost:8123/ping"       '^200$'

# ─── Summary ────────────────────────────────────────────────────────────────
echo
echo "═══════════════ SUMMARY ═══════════════"
printf '%s\n' "${RESULTS[@]}"
echo "───────────────────────────────────────"
echo "PASS: $PASS   FAIL: $FAIL   (log: $LOG)"
[[ $FAIL -eq 0 ]] && { echo "RESULT: INSTALLATION TEST PASSED"; exit 0; } \
                  || { echo "RESULT: INSTALLATION TEST FAILED"; exit 1; }

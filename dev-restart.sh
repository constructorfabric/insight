#!/usr/bin/env bash
# Quick restart of the Insight stack after WSL crash / Docker restart, or
# after `dev-down.sh` (which scales replicas to 0). Lightweight alternative
# to a full `dev-up.sh` re-run when no images need rebuilding.
#
# What it does:
#   1. local mode: ensure the Kind cluster's control-plane container is up.
#   2. wait for the cluster API server to respond.
#   3. scale every workload in the `insight` namespace back to replicas=1.
#   4. ensure CoreDNS still uses public DNS upstream (WSL workaround;
#      delegates to scripts/dev/patch-coredns-wsl.sh).
#   5. clean up Airbyte sync pods stuck in Failed/Succeeded.
#   6. (re-)start the Airbyte port-forward.
#
# What it does NOT do:
#   - re-build Docker images
#   - re-run `helm upgrade --install` (chart changes — use `dev-up.sh`)
#   - run `init.sh` (databases, connector registration — use `run-init.sh`)
#   - recreate the cluster if it's gone (delegates to `dev-up.sh`)
#
# Usage:
#   ./dev-restart.sh                  # default: --env local
#   ./dev-restart.sh --env virtuozzo  # remote env (just verify connectivity)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# ─── Parse --env ──────────────────────────────────────────────────────────
ENV_NAME="local"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --env)
      [[ -n "${2-}" ]] || { echo "ERROR: --env requires a value" >&2; exit 1; }
      ENV_NAME="$2"; shift 2 ;;
    --env=*)
      ENV_NAME="${1#*=}"
      [[ -n "$ENV_NAME" ]] || { echo "ERROR: --env= requires a value" >&2; exit 1; }
      shift ;;
    -h|--help)
      grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      exit 1 ;;
  esac
done

ENV_FILE="$ROOT_DIR/.env.${ENV_NAME}"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "ERROR: env file not found: $ENV_FILE" >&2
  exit 1
fi
set -a; source "$ENV_FILE"; set +a

CLUSTER_MODE="${CLUSTER_MODE:-local}"
CLUSTER_NAME="${CLUSTER_NAME:-insight}"
NAMESPACE="${INSIGHT_NAMESPACE:-${NAMESPACE:-insight}}"

echo "═══════════════════════════════════════════════════════════════"
echo "  Insight Platform — Restart"
echo "  Environment: ${ENV_NAME}   (${CLUSTER_MODE})"
echo "  Namespace:   ${NAMESPACE}"
echo "═══════════════════════════════════════════════════════════════"

# ─── Remote mode: verify connectivity only ────────────────────────────────
if [[ "$CLUSTER_MODE" != "local" ]]; then
  echo "Remote cluster — verifying connectivity..."
  kubectl cluster-info --request-timeout=15s \
    || { echo "ERROR: cannot reach cluster" >&2; exit 1; }
  echo "Cluster reachable. Run ./dev-up.sh --env $ENV_NAME to redeploy."
  exit 0
fi

# ─── Local Kind: bring the cluster back ──────────────────────────────────
command -v kind &>/dev/null || { echo "ERROR: kind required" >&2; exit 1; }
KUBECONFIG_PATH="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"

# Clean up exited Kind containers
for c in $(docker ps -a --filter "status=exited" --filter "name=kind" --format '{{.Names}}' 2>/dev/null); do
  echo "  Removing dead container: $c"
  docker rm -f "$c" 2>/dev/null || true
done

if ! kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
  echo "Cluster '${CLUSTER_NAME}' not found — falling back to ./dev-up.sh"
  exec "$ROOT_DIR/dev-up.sh" --env "$ENV_NAME"
fi

echo "=== Restarting Kind cluster '${CLUSTER_NAME}' ==="
docker start "${CLUSTER_NAME}-control-plane" >/dev/null 2>&1 || true

kind export kubeconfig --name "${CLUSTER_NAME}" --kubeconfig "${KUBECONFIG_PATH}" 2>/dev/null || true
export KUBECONFIG="${KUBECONFIG_PATH}"

echo "  Waiting for control-plane node Ready..."
if ! kubectl wait --for=condition=Ready node --all --timeout=60s &>/dev/null; then
  echo "  Cluster did not become Ready in 60s — falling back to ./dev-up.sh"
  exec "$ROOT_DIR/dev-up.sh" --env "$ENV_NAME"
fi

echo "  API server reachable. Scaling '${NAMESPACE}' workloads back to replicas=1..."
kubectl scale statefulset -n "$NAMESPACE" --all --replicas=1 2>/dev/null || true
kubectl scale deployment  -n "$NAMESPACE" --all --replicas=1 2>/dev/null || true

echo "  Waiting for key infra pods..."
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=mariadb     -n "$NAMESPACE" --timeout=120s 2>/dev/null \
  || echo "  WARNING: MariaDB not ready"
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=clickhouse  -n "$NAMESPACE" --timeout=120s 2>/dev/null \
  || echo "  WARNING: ClickHouse not ready"
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=redis       -n "$NAMESPACE" --timeout=120s 2>/dev/null \
  || echo "  WARNING: Redis not ready"

# Reapply CoreDNS WSL workaround (idempotent — no-op if already patched).
"$ROOT_DIR/scripts/dev/patch-coredns-wsl.sh"

# Clean up stale Airbyte replication-job pods (terminal phases only —
# phase!=Running would also match Pending and ContainerCreating).
for phase in Failed Succeeded; do
  kubectl delete pod -n "$NAMESPACE" --field-selector="status.phase=$phase" --force 2>/dev/null || true
done

# (Re-)start Airbyte port-forward in the background.
pkill -f 'port-forward.*airbyte' 2>/dev/null || true
nohup kubectl -n "$NAMESPACE" port-forward svc/airbyte-airbyte-server-svc 8001:8001 \
  >/dev/null 2>&1 &
disown

echo ""
echo "=== Restart complete ==="
echo "  KUBECONFIG: ${KUBECONFIG}"
echo "  Frontend:   http://localhost:${INGRESS_HTTP_PORT:-8000}"
echo "  API:        http://localhost:${INGRESS_HTTP_PORT:-8000}/api"
echo "  Airbyte:    http://localhost:8001"

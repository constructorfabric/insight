#!/usr/bin/env bash
# Insight platform — DEV bring-up from source.
#
# Use this when you work on the codebase: builds Docker images from src/,
# creates a local Kind cluster (or targets a dev-owned remote like virtuozzo),
# loads images into the cluster, and deploys all services.
#
# NOT for end-user installations. For customers / production-like installs
# from published chart artifacts, use:  deploy/scripts/install.sh
#
# Environment is selected with --env <name>. Configuration for each environment
# lives in .env.<name> at the repo root. See .env.local.example for the full
# contract.
#
# Components:
#   1. Cluster bootstrap (Kind create / external kubeconfig)
#   2. Ingress controller (optional, driven by INGRESS_INSTALL)
#   3. Ingestion (Airbyte, ClickHouse, Argo)
#   4. Backend (Analytics API, Identity Resolution, API Gateway)
#   5. Frontend (SPA)
#
# Usage:
#   ./dev-up.sh                         # --env local, all components
#   ./dev-up.sh --env virtuozzo         # remote cluster, all components
#   ./dev-up.sh --env virtuozzo app     # backend + frontend only
#   ./dev-up.sh ingestion               # only ingestion (default env=local)
#
# Valid components: all | ingestion | app | backend | frontend
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# ─── Argument parsing ─────────────────────────────────────────────────────
ENV_NAME="local"
COMPONENT="all"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --env)
      ENV_NAME="$2"
      shift 2
      ;;
    --env=*)
      ENV_NAME="${1#*=}"
      shift
      ;;
    -h|--help)
      grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      COMPONENT="$1"
      shift
      ;;
  esac
done

# ─── Load environment config ──────────────────────────────────────────────
ENV_FILE="$ROOT_DIR/.env.${ENV_NAME}"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "ERROR: env file not found: $ENV_FILE" >&2
  echo "       Copy .env.${ENV_NAME}.example to .env.${ENV_NAME} and fill it in." >&2
  exit 1
fi
# shellcheck source=/dev/null
set -a
source "$ENV_FILE"
set +a

# ─── Defaults (env file may override) ─────────────────────────────────────
CLUSTER_MODE="${CLUSTER_MODE:-local}"           # local | remote
CLUSTER_NAME="${CLUSTER_NAME:-insight}"
NAMESPACE="${NAMESPACE:-insight}"

IMAGE_REGISTRY="${IMAGE_REGISTRY:-}"            # empty = local-only images
IMAGE_TAG="${IMAGE_TAG:-local}"
IMAGE_PULL_POLICY="${IMAGE_PULL_POLICY:-IfNotPresent}"
IMAGE_PULL_SECRET="${IMAGE_PULL_SECRET:-}"
IMAGE_PLATFORM="${IMAGE_PLATFORM:-}"            # e.g., linux/amd64. Empty = native.
BUILD_IMAGES="${BUILD_IMAGES:-true}"
BUILD_AND_PUSH="${BUILD_AND_PUSH:-false}"
LOAD_IMAGES_INTO_KIND="${LOAD_IMAGES_INTO_KIND:-auto}"   # auto = yes iff CLUSTER_MODE=local

INGRESS_INSTALL="${INGRESS_INSTALL:-false}"
INGRESS_CLASS="${INGRESS_CLASS:-nginx}"
INGRESS_ENABLED="${INGRESS_ENABLED:-false}"     # chart-level ingress resource
INGRESS_MODE="${INGRESS_MODE:-hostPort}"        # hostPort | nodePort
INGRESS_HTTP_PORT="${INGRESS_HTTP_PORT:-80}"
INGRESS_HTTPS_PORT="${INGRESS_HTTPS_PORT:-443}"

AUTH_DISABLED="${AUTH_DISABLED:-false}"
OIDC_EXISTING_SECRET="${OIDC_EXISTING_SECRET:-}"
OIDC_ISSUER="${OIDC_ISSUER:-}"
OIDC_CLIENT_ID="${OIDC_CLIENT_ID:-}"
OIDC_REDIRECT_URI="${OIDC_REDIRECT_URI:-}"
OIDC_AUDIENCE="${OIDC_AUDIENCE:-}"

# Identity-service default tenant for header-less callers. Each developer's
# MariaDB has rows under a specific `insight_tenant_id` — pin it here so
# `GET /v1/persons/{email}` works without sending `X-Insight-Tenant-Id`.
# Empty = identity requires the header on every request (prod mode).
IDENTITY_TENANT_DEFAULT_ID="${IDENTITY_TENANT_DEFAULT_ID:-}"

# ─── Sanity ───────────────────────────────────────────────────────────────
if [[ "$AUTH_DISABLED" != "true" && -z "$OIDC_EXISTING_SECRET" ]]; then
  : "${OIDC_ISSUER:?ERROR: OIDC_ISSUER is required — set it in $ENV_FILE or use OIDC_EXISTING_SECRET}"
  : "${OIDC_AUDIENCE:?ERROR: OIDC_AUDIENCE is required (e.g. api://insight) — set it in $ENV_FILE or use OIDC_EXISTING_SECRET}"
  : "${OIDC_CLIENT_ID:?ERROR: OIDC_CLIENT_ID is required — set it in $ENV_FILE or use OIDC_EXISTING_SECRET}"
  : "${OIDC_REDIRECT_URI:?ERROR: OIDC_REDIRECT_URI is required — set it in $ENV_FILE or use OIDC_EXISTING_SECRET}"
fi

echo "═══════════════════════════════════════════════════════════════"
echo "  Insight Platform"
echo "  Environment: ${ENV_NAME}   (${CLUSTER_MODE})"
echo "  Component:   ${COMPONENT}"
echo "  Namespace:   ${NAMESPACE}"
echo "  Image:       ${IMAGE_REGISTRY:-<local>}/insight-*:${IMAGE_TAG}"
echo "═══════════════════════════════════════════════════════════════"

declare -A INSTALL_HINT=(
  [kubectl]="https://kubernetes.io/docs/tasks/tools/  (Ubuntu: 'snap install kubectl --classic')"
  [helm]="https://helm.sh/docs/intro/install/  (Ubuntu: 'snap install helm --classic')"
  [docker]="Docker Desktop or Docker Engine — https://docs.docker.com/engine/install/"
  [kind]="https://kind.sigs.k8s.io/docs/user/quick-start/#installation  (Linux: 'go install sigs.k8s.io/kind@latest' or curl-install)"
)
require_cmd() {
  local cmd="$1" purpose="$2"
  if ! command -v "$cmd" &>/dev/null; then
    echo "ERROR: '$cmd' is required ($purpose) but not found in PATH." >&2
    echo "       Install: ${INSTALL_HINT[$cmd]:-see project README}" >&2
    echo "       After install, re-run: $0 $*" >&2
    exit 1
  fi
}
for cmd in kubectl helm docker; do
  require_cmd "$cmd" "core dependency"
done

# ─── Cluster bootstrap ────────────────────────────────────────────────────
if [[ "$CLUSTER_MODE" == "local" ]]; then
  require_cmd kind "CLUSTER_MODE=local — Kind manages the local cluster"
  KUBECONFIG_PATH="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"
  if ! kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
    echo "=== Creating Kind cluster '${CLUSTER_NAME}' ==="
    kind create cluster --config k8s/kind-config.yaml
  elif ! docker ps --format '{{.Names}}' | grep -q "^${CLUSTER_NAME}-control-plane$"; then
    echo "=== Starting Kind cluster ==="
    docker start "${CLUSTER_NAME}-control-plane"
    # Wait for control plane to actually be Ready instead of a fixed sleep —
    # flaky on slow machines, wasteful on fast ones.
    kind export kubeconfig --name "${CLUSTER_NAME}" --kubeconfig "${HOME}/.kube/insight.kubeconfig" 2>/dev/null || true
    KUBECONFIG="${HOME}/.kube/insight.kubeconfig" \
      kubectl wait --for=condition=Ready node --all --timeout=60s
  fi
  kind export kubeconfig --name "${CLUSTER_NAME}" --kubeconfig "${KUBECONFIG_PATH}" 2>/dev/null || true
  export KUBECONFIG="${KUBECONFIG_PATH}"

  # WSL/Kind DNS workaround: replace CoreDNS upstream `/etc/resolv.conf` with
  # public DNS so connector pods can reach external APIs (api.anthropic.com,
  # api.zoom.us, login.microsoftonline.com, …). Idempotent; opt out with
  # SKIP_COREDNS_PATCH=1; override upstreams via DNS_UPSTREAMS.
  "$ROOT_DIR/scripts/dev/patch-coredns-wsl.sh"
else
  # remote: KUBECONFIG must already be set via env file or shell
  : "${KUBECONFIG:?ERROR: KUBECONFIG must be set for CLUSTER_MODE=remote (set it in $ENV_FILE)}"
  if [[ "$KUBECONFIG" != /* ]]; then
    KUBECONFIG="$ROOT_DIR/$KUBECONFIG"
  fi
  [[ -f "$KUBECONFIG" ]] || { echo "ERROR: kubeconfig not found: $KUBECONFIG" >&2; exit 1; }
  export KUBECONFIG
fi
echo "  KUBECONFIG=${KUBECONFIG}"

# Resolve whether to load images into Kind
if [[ "$LOAD_IMAGES_INTO_KIND" == "auto" ]]; then
  [[ "$CLUSTER_MODE" == "local" ]] && LOAD_IMAGES_INTO_KIND=true || LOAD_IMAGES_INTO_KIND=false
fi

# ─── Ingress controller ───────────────────────────────────────────────────
if [[ "$INGRESS_INSTALL" == "true" ]]; then
  echo "=== Installing ingress-nginx (${INGRESS_MODE}) ==="
  helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx 2>/dev/null || true
  helm repo update ingress-nginx >/dev/null
  INGRESS_ARGS=(
    --namespace ingress-nginx --create-namespace
    --set controller.watchIngressWithoutClass=true
    --set controller.ingressClassResource.default=true
  )
  case "$INGRESS_MODE" in
    hostPort)
      INGRESS_ARGS+=(
        --set controller.kind=DaemonSet
        --set controller.hostPort.enabled=true
        --set controller.hostPort.ports.http="$INGRESS_HTTP_PORT"
        --set controller.hostPort.ports.https="$INGRESS_HTTPS_PORT"
        --set controller.service.type=ClusterIP
      )
      ;;
    nodePort)
      INGRESS_ARGS+=(
        --set controller.service.type=NodePort
        --set controller.service.nodePorts.http="$INGRESS_HTTP_PORT"
        --set controller.service.nodePorts.https="$INGRESS_HTTPS_PORT"
      )
      ;;
    *)
      echo "ERROR: unknown INGRESS_MODE: $INGRESS_MODE (hostPort|nodePort)" >&2
      exit 1
      ;;
  esac
  helm upgrade --install ingress-nginx ingress-nginx/ingress-nginx \
    "${INGRESS_ARGS[@]}" --wait --timeout 5m
fi

# ─── Namespace ────────────────────────────────────────────────────────────
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

# ─── Image build (dev-only) ───────────────────────────────────────────────
# For the dev loop we build container images from src/ and load them into
# Kind. Prod customers use pre-published images from ghcr.io.

image_tag_for() {
  local svc="$1"
  # Per-service override: API_GATEWAY_IMAGE_TAG, ANALYTICS_API_IMAGE_TAG, etc.
  local var_name; var_name="$(echo "$svc" | tr '[:lower:]-' '[:upper:]_')_IMAGE_TAG"
  echo "${!var_name:-$IMAGE_TAG}"
}

image_ref() {
  local svc="$1"
  echo "${IMAGE_REGISTRY:+$IMAGE_REGISTRY/}insight-${svc}:$(image_tag_for "$svc")"
}

build_and_load_image() {
  local svc="$1" dockerfile="$2" ctx="${3:-src/backend/}"
  local full; full=$(image_ref "$svc")

  if [[ "$BUILD_IMAGES" == "true" ]]; then
    if [[ -n "$IMAGE_PLATFORM" ]]; then
      [[ -n "$IMAGE_REGISTRY" ]] || { echo "ERROR: IMAGE_PLATFORM requires IMAGE_REGISTRY" >&2; exit 1; }
      echo "  Building ${full} for ${IMAGE_PLATFORM} (buildx + push)..."
      docker buildx build --platform "$IMAGE_PLATFORM" -t "$full" -f "$dockerfile" --push "$ctx"
    else
      echo "  Building ${full}..."
      docker build -t "$full" -f "$dockerfile" "$ctx"
      if [[ -n "$IMAGE_REGISTRY" && "$BUILD_AND_PUSH" == "true" ]]; then
        echo "  Pushing ${full}..."
        docker push "$full"
      fi
    fi
  fi
  if [[ "$LOAD_IMAGES_INTO_KIND" == "true" ]]; then
    echo "  Loading ${full} into Kind..."
    kind load docker-image "$full" --name "${CLUSTER_NAME}"
  fi
}

# Frontend image coordinates — defined OUTSIDE the `if != ingestion`
# block because the DEV_VALUES heredoc below references `${FE_REPO}` /
# `${FE_TAG}` unconditionally. Under `set -u`, an ingestion-only run
# would otherwise crash with `FE_REPO: unbound variable` after the
# toolbox/jira-enrich builds succeed but before the values file is
# generated.
FE_REPO="${FE_IMAGE_REPOSITORY:-ghcr.io/cyberfabric/insight-front}"
FE_TAG="${FE_IMAGE_TAG:-$(image_tag_for frontend)}"
FE_IMAGE="${FE_REPO}:${FE_TAG}"

# App services are MANDATORY components of the umbrella (no enabled-flag),
# so whenever we install the umbrella — including `frontend` or `backend`
# component runs that trigger helm upgrade — every image must be present
# in the cluster. Otherwise backend pods land in ImagePullBackOff.
if [[ "$COMPONENT" != "ingestion" ]]; then
  echo "=== Building backend images ==="
  build_and_load_image analytics-api src/backend/services/analytics-api/Dockerfile
  # The .NET 9 identity service uses its OWN build context (the service
  # folder), unlike the Rust services which share `src/backend/` as context.
  build_and_load_image identity      src/backend/services/identity/Dockerfile src/backend/services/identity/
  build_and_load_image api-gateway   src/backend/services/api-gateway/Dockerfile

  # Frontend — prefer a local build from the neighbouring insight-front
  # checkout. Published images are currently amd64-only and fail to load
  # on arm64 Kind nodes (even with Rosetta, kubelet rejects manifest
  # index mismatches). A local native-arch build is reliable and matches
  # what the dev just edited.
  # No `:latest` default — tag is the dev image_tag_for() output, which
  # is a deterministic dev tag (`dev`, derived from commit / mtime).
  # Locate the insight-front checkout. The committed `insight-front_symlink`
  # only resolves in the primary worktree; under .claude/worktrees/<branch>
  # we fall back to git's worktree list to find the main repo's sibling.
  FE_SRC="${FE_SRC:-}"
  if [[ -z "$FE_SRC" ]]; then
    for candidate in \
      "$ROOT_DIR/insight-front_symlink" \
      "$(git worktree list 2>/dev/null | head -1 | awk '{print $1}')/insight-front_symlink" \
      "$(cd "$ROOT_DIR" && git rev-parse --show-toplevel 2>/dev/null)/../insight-front"
    do
      if [[ -n "$candidate" && -d "$candidate" && -f "$candidate/Dockerfile" ]]; then
        FE_SRC="$candidate"
        break
      fi
    done
  fi

  # Honour BUILD_IMAGES=false: the caller has the FE image already (e.g.
  # local registry warm-cache, manual pre-pull) and does not want
  # `dev-up.sh` to rebuild from the neighbouring insight-front checkout
  # OR to re-pull from ghcr — issue #275 B8. Skip the build/pull here;
  # the kind-load step below still tries to load the existing local
  # image into the cluster, which is cheap and idempotent.
  if [[ "$BUILD_IMAGES" == "true" ]]; then
    if [[ -n "$FE_SRC" && -d "$FE_SRC" && -f "$FE_SRC/Dockerfile" ]]; then
      echo "=== Building Frontend image from $FE_SRC ==="
      docker build -t "$FE_IMAGE" -f "$FE_SRC/Dockerfile" "$FE_SRC"
    else
      echo "=== Pulling Frontend image ==="
      case "$(uname -m)" in
        arm64|aarch64) FE_PLATFORM="linux/arm64" ;;
        *)             FE_PLATFORM="linux/amd64" ;;
      esac
      if ! docker pull --platform "$FE_PLATFORM" "$FE_IMAGE" 2>/dev/null; then
        echo "  (no $FE_PLATFORM manifest, falling back to linux/amd64 via emulation)"
        docker pull --platform linux/amd64 "$FE_IMAGE"
      fi
    fi
  else
    echo "=== Skipping Frontend build/pull (BUILD_IMAGES=false) ==="
  fi
  [[ "$LOAD_IMAGES_INTO_KIND" == "true" ]] && kind load docker-image "$FE_IMAGE" --name "${CLUSTER_NAME}"
fi

# Ingestion-template images. These run inside Argo WorkflowTemplates
# (charts/insight/templates/ingestion/*.yaml) when
# `ingestion.templates.enabled=true`:
#   - insight-toolbox       — dbt + helper CLI; runs all dbt-run workflows
#   - insight-jira-enrich   — Rust binary that augments bronze_jira between
#                             Airbyte sync and silver-layer dbt models
# We build them on `dev-up.sh all` and `dev-up.sh ingestion` so the local
# stack matches production behaviour out of the box. Skip when only
# building app/backend/frontend — those runs neither install Argo
# templates nor need these images.
#
# The jira-enrich image tag is read from the jira descriptor's
# `images.enrich.image` field (per ADR-0016) — the same value reconcile
# will hand to the WorkflowTemplate at runtime — and the build script
# tags + kind-loads that exact ref. Because `tt-enrich-jira-run.yaml` uses
# `imagePullPolicy: IfNotPresent`, the pod uses the locally-cached image
# without contacting the remote registry. To run a different binary
# locally, edit `images.enrich.image` in the descriptor and rerun dev-up.
TOOLBOX_IMAGE_REF="${INGESTION_TOOLBOX_IMAGE:-insight-toolbox:local}"
JIRA_DESCRIPTOR="$ROOT_DIR/src/ingestion/connectors/task-tracking/jira/descriptor.yaml"
JIRA_ENRICH_IMAGE_REF="$(python3 \
  "$ROOT_DIR/src/ingestion/reconcile-connectors/python/parse_descriptor.py" \
  --descriptor "$JIRA_DESCRIPTOR" --field images.enrich.image)"
[[ -n "$JIRA_ENRICH_IMAGE_REF" ]] || {
  echo "ERROR: images.enrich.image missing from $JIRA_DESCRIPTOR — set it before running dev-up." >&2
  exit 1
}
if [[ "$COMPONENT" == "all" || "$COMPONENT" == "ingestion" ]] && [[ "$BUILD_IMAGES" == "true" ]]; then
  echo "=== Building ingestion-template images ==="
  TOOLBOX_IMAGE="$TOOLBOX_IMAGE_REF" "$ROOT_DIR/src/ingestion/tools/toolbox/build.sh"
  JIRA_ENRICH_IMAGE="$JIRA_ENRICH_IMAGE_REF" KIND_CLUSTER="$CLUSTER_NAME" \
    "$ROOT_DIR/src/ingestion/connectors/task-tracking/jira/enrich/build.sh"
fi

# ─── Generate dev overrides for umbrella ──────────────────────────────────
# The canonical installer reads the umbrella values.yaml plus overrides.
# We produce a single tempfile with env-derived values; the standing
# `deploy/values-dev.yaml` overlay (eval-grade credentials that the
# canonical chart leaves empty) is passed as the first -f via
# INSIGHT_VALUES_FILES, so helm merges them in order.
DEV_VALUES=$(mktemp)
trap 'rm -f "$DEV_VALUES"' EXIT

ANALYTICS_IMG=$(image_ref analytics-api)
# `identity` image is the .NET service (legacy Rust stub retired —
# build_and_load_image identity above points at the C# Dockerfile).
IDENTITY_IMG=$(image_ref identity)
GATEWAY_IMG=$(image_ref api-gateway)

# Emit "repo tag" on a single line with a trailing newline — `read` aborts
# under `set -e` if EOF comes before a newline.
split_image() { printf '%s %s\n' "${1%:*}" "${1##*:}"; }
read -r GW_REPO GW_TAG_VAL < <(split_image "$GATEWAY_IMG")
read -r AN_REPO AN_TAG_VAL < <(split_image "$ANALYTICS_IMG")
read -r ID_REPO ID_TAG_VAL < <(split_image "$IDENTITY_IMG")

cat > "$DEV_VALUES" <<EOF
# Auto-generated by dev-up.sh — do not edit (values derived from .env.${ENV_NAME})
apiGateway:
  image:
    repository: "${GW_REPO}"
    tag: "${GW_TAG_VAL}"
    pullPolicy: "${IMAGE_PULL_POLICY}"
  authDisabled: ${AUTH_DISABLED}
  oidc:
    existingSecret: "${OIDC_EXISTING_SECRET}"
    issuer: "${OIDC_ISSUER}"
    audience: "${OIDC_AUDIENCE}"
    clientId: "${OIDC_CLIENT_ID}"
    redirectUri: "${OIDC_REDIRECT_URI}"
  ingress:
    enabled: ${INGRESS_ENABLED}
    className: "${INGRESS_CLASS}"
  gateway:
    enableDocs: true
analyticsApi:
  image:
    repository: "${AN_REPO}"
    tag: "${AN_TAG_VAL}"
    pullPolicy: "${IMAGE_PULL_POLICY}"
identity:
  image:
    repository: "${ID_REPO}"
    tag: "${ID_TAG_VAL}"
    pullPolicy: "${IMAGE_PULL_POLICY}"
  tenantDefaultId: "${IDENTITY_TENANT_DEFAULT_ID}"
frontend:
  image:
    # Frontend image is built from local source by build_and_load_frontend()
    # below — repo + tag are computed from \$(image_ref frontend), no
    # \`:latest\` fallback (the chart \`required\`s tag).
    repository: "${FE_REPO}"
    tag: "${FE_TAG}"
    pullPolicy: "${IMAGE_PULL_POLICY}"
  ingress:
    enabled: ${INGRESS_ENABLED}
    className: "${INGRESS_CLASS}"
ingestion:
  toolboxImage:    "${TOOLBOX_IMAGE_REF}"
EOF

if [[ -n "$IMAGE_PULL_SECRET" ]]; then
  cat >> "$DEV_VALUES" <<EOF
global:
  imagePullSecrets:
    - name: "${IMAGE_PULL_SECRET}"
EOF
fi

# Detect whether Argo CRDs are present. Umbrella ships WorkflowTemplate
# objects which require the Argo CRDs; if Argo was not installed yet
# (e.g. the dev runs `./dev-up.sh backend`), the umbrella would fail with
# `no matches for kind "WorkflowTemplate"`. Skip ingestion templates in
# that case — running `./dev-up.sh ingestion` or `all` installs them.
ARGO_CRD_GUARD=""
if ! kubectl get crd workflowtemplates.argoproj.io >/dev/null 2>&1; then
  ARGO_CRD_GUARD="--set ingestion.templates.enabled=false"
fi

# Ordered list of values files passed to the umbrella. dev overlay first
# (base eval credentials), env-derived override file second, then anything
# the caller pre-set in INSIGHT_VALUES_FILES last so it wins on conflicts.
# Without the trailing append, exporting INSIGHT_VALUES_FILES before
# `dev-up.sh` had no effect — the script overwrote it unconditionally
# (issue #275 B7).
_BASE_VALUES_FILES="$ROOT_DIR/deploy/values-dev.yaml:$DEV_VALUES"
INSIGHT_VALUES_FILES="${_BASE_VALUES_FILES}${INSIGHT_VALUES_FILES:+:$INSIGHT_VALUES_FILES}"

# ─── Delegate to canonical installers ─────────────────────────────────────
case "$COMPONENT" in
  all)
    INSIGHT_NAMESPACE="$NAMESPACE" \
      INSIGHT_VALUES_FILES="$INSIGHT_VALUES_FILES" \
      DEV_MODE=1 \
      "$ROOT_DIR/deploy/scripts/install.sh"
    ;;
  ingestion)
    INSIGHT_NAMESPACE="$NAMESPACE" DEV_MODE=1 "$ROOT_DIR/deploy/scripts/install-airbyte.sh"
    INSIGHT_NAMESPACE="$NAMESPACE" DEV_MODE=1 "$ROOT_DIR/deploy/scripts/install-argo.sh"
    ;;
  app|backend|frontend)
    # The umbrella deploys EVERYTHING: infra + backend + frontend. For
    # backend-only or frontend-only runs we keep the full deploy —
    # helm upgrade is idempotent and app services are mandatory
    # components of the umbrella (no per-service enable flag).
    SKIP_AIRBYTE=1 SKIP_ARGO=1 \
      INSIGHT_NAMESPACE="$NAMESPACE" \
      INSIGHT_VALUES_FILES="$INSIGHT_VALUES_FILES" \
      HELM_EXTRA_ARGS="$ARGO_CRD_GUARD" \
      "$ROOT_DIR/deploy/scripts/install.sh"
    ;;
  *)
    echo "ERROR: unknown component: $COMPONENT (expected: all|ingestion|app|backend|frontend)" >&2
    exit 1
    ;;
esac

# ─── Port-forwards (local only) ────────────────────────────────────────
# Every service that a developer would want to hit gets a local port.
# All subcharts live in the release namespace, so one loop is enough.
PF_PIDS=()
if [[ "$CLUSTER_MODE" == "local" ]]; then
  echo "=== Starting port-forwards ==="
  # Track the PIDs of the port-forwards we spawn so we can stop only
  # those — `pkill -f port-forward.*-n $NAMESPACE` matches any kubectl
  # port-forward whose argv mentions the namespace, including unrelated
  # PFs the developer may have started in another terminal.
  pf() { # svc host-port:svc-port
    kubectl -n "$NAMESPACE" port-forward "svc/$1" "$2" >/dev/null 2>&1 &
    PF_PIDS+=("$!")
  }

  # Stack UI / API (umbrella)
  pf insight-frontend              8003:80
  pf insight-api-gateway           8080:8080
  pf insight-clickhouse            8123:8123
  pf insight-mariadb               3306:3306
  pf insight-redis-master          6379:6379

  # Airbyte
  pf airbyte-airbyte-webapp-svc    8002:80
  pf airbyte-airbyte-server-svc    8001:8001

  # Argo Workflows UI (chart's server SVC)
  pf argo-workflows-server         2746:2746

  echo "  (port-forward PIDs: ${PF_PIDS[*]} — kill with \`kill ${PF_PIDS[*]}\`)"
  sleep 2
fi

# ─── Credentials ───────────────────────────────────────────────────────
# Read resolved values from the auto-generated Secret (see
# charts/insight/templates/secrets.yaml). Works only after the umbrella
# install completes; for `ingestion` component it has not been installed.
secret_value() {
  kubectl -n "$NAMESPACE" get secret "$1" -o jsonpath="{.data.$2}" 2>/dev/null | base64 -d 2>/dev/null || true
}

# NB: chart emits a FIXED Secret name (`insight-db-creds`) regardless of
# the umbrella release name — there is one Insight install per namespace.
CH_PASS=$(secret_value insight-db-creds clickhouse-password)
MDB_PASS=$(secret_value insight-db-creds mariadb-password)
MDB_ROOT=$(secret_value insight-db-creds mariadb-root-password)
REDIS_PASS=$(secret_value insight-db-creds redis-password)
AB_PASS=$(secret_value airbyte-auth-secrets instance-admin-password)

# ─── Summary ──────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  KUBECONFIG: ${KUBECONFIG}"
echo "  Namespace:  ${NAMESPACE}"
echo ""
echo "  URL                           Target"
echo "  ───────────────────────────────────────────────────────────"
if [[ "$CLUSTER_MODE" == "local" ]]; then
  echo "  http://localhost:8003         Frontend (SPA)"
  echo "  http://localhost:8080         API Gateway (health + /api)"
  echo "  http://localhost:8002         Airbyte UI"
  echo "  http://localhost:8001         Airbyte API"
  echo "  http://localhost:2746         Argo Workflows UI"
  echo "  http://localhost:8123         ClickHouse HTTP"
  echo "        localhost:3306          MariaDB"
  echo "        localhost:6379          Redis"
else
  echo "  Remote cluster — no local port-forwards."
  echo "  Expose services via ingress / kubectl port-forward manually."
fi
echo ""
echo "  Credentials"
echo "  ───────────────────────────────────────────────────────────"
if [[ "$CLUSTER_MODE" == "local" || "${SHOW_CREDS:-}" == "1" ]]; then
  [[ -n "$AB_PASS" ]]    && echo "  Airbyte  ${AIRBYTE_SETUP_EMAIL:-admin}  / $AB_PASS"
  [[ -n "$CH_PASS" ]]    && echo "  CH       insight             / $CH_PASS"
  [[ -n "$MDB_PASS" ]]   && echo "  MariaDB  insight             / $MDB_PASS"
  [[ -n "$MDB_ROOT" ]]   && echo "  MariaDB  root                / $MDB_ROOT"
  [[ -n "$REDIS_PASS" ]] && echo "  Redis    default             / $REDIS_PASS"
else
  # For remote envs (virtuozzo, prod-like) — DO NOT dump passwords into
  # the developer's terminal scrollback. Print only how to fetch them.
  echo "  (remote cluster — passwords not printed; pass SHOW_CREDS=1 to override)"
  echo "  Fetch with:"
  echo "    kubectl -n ${NAMESPACE} get secret insight-db-creds -o yaml"
  echo "    kubectl -n ${NAMESPACE} get secret airbyte-auth-secrets -o jsonpath='{.data.instance-admin-password}' | base64 -d"
fi
echo ""
echo "  (passwords auto-generated on first install, stored in Secret"
echo "   'insight-db-creds' — stable across upgrades.)"
echo "═══════════════════════════════════════════════════════════════"

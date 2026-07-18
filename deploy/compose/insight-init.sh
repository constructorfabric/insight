#!/usr/bin/env bash
# Insight — shared first-run wizard.
#
# Generates the env-specific config file for one of two bring-up paths:
#
#   --target=compose    Writes .env.compose at the insight repo root.
#                       Invoked by `./dev-compose.sh up` when .env.compose
#                       is missing.
#
#   --target=k8s-local  Writes deploy/gitops/environments/local/inventory.yaml
#                       and populates deploy/gitops/secrets-store.yaml.
#                       Invoked by `make deploy ENV=local` when inventory.yaml
#                       is missing.
#
# Behavior:
#   - Common questions (MariaDB / ClickHouse / tenant / dev email) ask once.
#   - Target-specific extras follow.
#   - Errors out hard on non-TTY stdin: the wizard is interactive only.
#   - Errors out hard if the target output file already exists: delete it
#     first to re-run.
#
# Why this lives in deploy/compose/ rather than its own top-level directory:
#   The wizard descends from dev-compose.sh's first-run prompts. Keeping it
#   alongside the compose stack avoids a separate top-level dir; the k8s caller
#   reaches it via `../compose/insight-init.sh` from deploy/gitops/.

set -euo pipefail

# ──────────────────────────────────────────────────────────────────────
# Resolve repo root from this script's location, regardless of CWD.
# Layout: <ROOT_DIR>/deploy/compose/insight-init.sh
# ──────────────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

usage() {
  cat <<'EOF'
usage: insight-init.sh --target=compose|k8s-local

Targets:
  --target=compose     Generate .env.compose for the docker-compose stack.
  --target=k8s-local   Generate deploy/gitops/environments/local/
                       inventory.yaml + populate secrets-store.yaml for the
                       k8s gitops stack.

The wizard is interactive only. Run from a terminal.
EOF
}

TARGET=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target=*) TARGET="${1#*=}"; shift ;;
    --target)
      TARGET="${2:-}"
      shift
      [[ $# -gt 0 ]] && shift
      ;;
    -h|--help)  usage; exit 0 ;;
    *) echo "ERROR: unknown arg: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$TARGET" in
  compose|k8s-local) ;;
  "")  echo "ERROR: --target is required" >&2; usage >&2; exit 2 ;;
  *)   echo "ERROR: unknown target: $TARGET" >&2; usage >&2; exit 2 ;;
esac

if [[ ! -t 0 ]]; then
  echo "ERROR: insight-init.sh needs an interactive shell (stdin is not a TTY)." >&2
  echo "       Run from a terminal — there is no non-interactive fallback." >&2
  exit 1
fi

# ──────────────────────────────────────────────────────────────────────
# IO helpers — lifted verbatim from dev-compose.sh so behavior matches.
# ──────────────────────────────────────────────────────────────────────

# ask <prompt> <default> — print prompt, read one line, echo answer (or
# default on empty input). Prompts go to stderr so the captured stdout
# stays clean.
ask() {
  local prompt="$1" default="${2:-}" answer
  if [[ -n "$default" ]]; then
    printf '%s [%s]: ' "$prompt" "$default" >&2
  else
    printf '%s: ' "$prompt" >&2
  fi
  read -r answer
  [[ -z "$answer" ]] && answer="$default"
  printf '%s' "$answer"
}

# ask_secret <prompt> — read a password without echoing it. No default.
ask_secret() {
  local prompt="$1" answer
  printf '%s: ' "$prompt" >&2
  read -rs answer
  printf '\n' >&2
  printf '%s' "$answer"
}

# ask_yes_no <prompt> <default y|n> — loop until a yes/no answer; return
# 0 for yes, 1 for no. Default is taken when the user hits Enter.
ask_yes_no() {
  local prompt="$1" default="${2:-y}" answer hint
  if [[ "$default" == "y" ]]; then hint="Y/n"; else hint="y/N"; fi
  while true; do
    printf '%s [%s]: ' "$prompt" "$hint" >&2
    read -r answer
    [[ -z "$answer" ]] && answer="$default"
    case "$(printf '%s' "$answer" | tr '[:upper:]' '[:lower:]')" in
      y|yes) return 0 ;;
      n|no)  return 1 ;;
      *) echo "  Please answer y or n." >&2 ;;
    esac
  done
}

# update_env_var <file> <key> <value> — replace `KEY=...` in <file>, or
# append a new line if the key doesn't exist. Portable across BSD (mac)
# and GNU sed.
update_env_var() {
  local file="$1" key="$2" value="$3" escaped tmp
  escaped=$(printf '%s' "$value" | sed -e 's/[\\&|]/\\&/g')
  if grep -qE "^[[:space:]]*${key}=" "$file" 2>/dev/null; then
    tmp=$(mktemp)
    sed -E "s|^[[:space:]]*${key}=.*|${key}=${escaped}|" "$file" > "$tmp"
    mv "$tmp" "$file"
  else
    printf '%s=%s\n' "$key" "$value" >> "$file"
  fi
}

# Warn loudly when the user pastes localhost as an "external" DB host —
# inside a container, that points at the container itself.
warn_localhost_host() {
  local host="$1" label="$2"
  case "$host" in
    localhost|127.0.0.1|::1)
      echo "  WARN: '$host' resolves to the container itself, not your host." >&2
      echo "        For a $label running on the docker host, use" >&2
      echo "        host.docker.internal (Mac/Windows) or your LAN IP." >&2
      ;;
  esac
}

# validate_mariadb host port user pass — `mariadb -e "SELECT 1"` via a
# transient mariadb container. Returns 0 on success.
validate_mariadb() {
  local host="$1" port="$2" user="$3" pass="$4"
  echo "  Probing MariaDB at ${host}:${port}..." >&2
  if docker run --rm mariadb:11.4 mariadb \
       -h "$host" -P "$port" -u "$user" "--password=$pass" \
       -e "SELECT 1" >/dev/null 2>&1; then
    echo "  MariaDB OK." >&2
    return 0
  fi
  echo "  ERROR: could not connect to MariaDB at ${host}:${port} as ${user}." >&2
  return 1
}

# validate_clickhouse host http_port user pass db — SELECT 1 via the HTTP
# interface using host-side curl. Returns 0 on success.
validate_clickhouse() {
  local host="$1" port="$2" user="$3" pass="$4" db="$5"
  echo "  Probing ClickHouse at ${host}:${port}..." >&2
  if curl -sf -u "${user}:${pass}" \
       --data-urlencode "query=SELECT 1" \
       --data-urlencode "database=${db}" \
       "http://${host}:${port}/" >/dev/null 2>&1; then
    echo "  ClickHouse OK." >&2
    return 0
  fi
  echo "  ERROR: could not connect to ClickHouse at ${host}:${port} as ${user}." >&2
  return 1
}

# ──────────────────────────────────────────────────────────────────────
# Cross-OS tooling preflight.
# ──────────────────────────────────────────────────────────────────────

_pkg_hint_macos=()
_pkg_hint_linux=()
_pkg_hint_url=()

# require <name> [macos-hint] [linux-hint] [url]
require() {
  local cmd="$1" mh="${2:-brew install $1}" lh="${3:-sudo apt-get install $1}" url="${4:-}"
  command -v "$cmd" >/dev/null 2>&1 && return 0
  echo "  MISSING: $cmd" >&2
  case "$(uname -s)" in
    Darwin) echo "    install: $mh" >&2 ;;
    Linux)
      if grep -qi microsoft /proc/version 2>/dev/null; then
        echo "    install (WSL): $lh" >&2
      else
        echo "    install: $lh   # or your distro's equivalent" >&2
      fi ;;
    *) [[ -n "$url" ]] && echo "    see: $url" >&2 || echo "    install via your OS package manager" >&2 ;;
  esac
  return 1
}

preflight_compose() {
  echo "--- Tooling preflight (compose) ---" >&2
  local missing=0
  require docker         "brew install --cask docker"  "sudo apt-get install docker.io"  "https://docs.docker.com/engine/install/" || missing=$((missing+1))
  if ! docker compose version >/dev/null 2>&1; then
    echo "  MISSING: docker compose (v2 plugin)" >&2
    echo "    upgrade Docker Desktop / install the docker-compose-plugin package" >&2
    missing=$((missing+1))
  fi
  [[ $missing -gt 0 ]] && { echo "  Resolve the missing tools and re-run." >&2; exit 1; }
  echo "  OK." >&2
  echo "" >&2
}

preflight_k8s() {
  echo "--- Tooling preflight (k8s-local) ---" >&2
  local missing=0
  require kubectl  "brew install kubectl"        "sudo apt-get install kubectl"        "https://kubernetes.io/docs/tasks/tools/" || missing=$((missing+1))
  require helm     "brew install helm"           "sudo apt-get install helm"           "https://helm.sh/docs/intro/install/"     || missing=$((missing+1))
  require kubeseal "brew install kubeseal"       "see kubeseal release page"           "https://github.com/bitnami-labs/sealed-secrets/releases" || missing=$((missing+1))
  require yq       "brew install yq"             "sudo snap install yq"                "https://github.com/mikefarah/yq#install" || missing=$((missing+1))
  require jq       "brew install jq"             "sudo apt-get install jq"             "https://jqlang.github.io/jq/download/"   || missing=$((missing+1))

  # Block helm v4.2.1 — known --wait regression that hangs the full
  # --timeout on every fast hook-resource deletion. Trips bootstrap-*
  # and system-* steps that use `before-hook-creation` lifecycle hooks
  # (ingress-nginx admission, cert-manager startupapicheck, etc.).
  # See helm/helm#32214; #32230 is the proposed revert. Pin to v4.2.0
  # or v3.x until v4.2.2+ ships.
  if command -v helm >/dev/null 2>&1; then
    local helm_ver
    helm_ver="$(helm version --short 2>/dev/null | sed 's/+.*//; s/^v//')"
    if [[ "$helm_ver" == "4.2.1" ]]; then
      echo "  BAD: helm $helm_ver has the --wait hook-deletion regression" >&2
      echo "       https://github.com/helm/helm/issues/32214" >&2
      echo "       Pin to v4.2.0 (https://get.helm.sh/) or v3.21.x until v4.2.2+ ships." >&2
      missing=$((missing+1))
    fi
  fi

  [[ $missing -gt 0 ]] && { echo "  Resolve the missing tools and re-run." >&2; exit 1; }
  echo "  OK." >&2
  echo "" >&2
}

# ──────────────────────────────────────────────────────────────────────
# Shared prompts — same questions for both targets.
# ──────────────────────────────────────────────────────────────────────
#
# Outputs assigned to shell variables for the target writers to consume.
# (Bash 3.2 lacks associative arrays, so plain globals it is.)

# Sentinel UUID used by Insight when a default tenant is needed.
TENANT_DEFAULT_UUID="00000000-df51-5b42-9538-d2b56b7ee953"

ask_shared() {
  cat >&2 <<EOF

=== Insight first-run wizard (target: $TARGET) ===

Collects the values needed to generate the target config. Press Enter
to accept the default shown in [brackets].

EOF

  # ── MariaDB ───────────────────────────────────────────────────────
  echo "--- MariaDB ---" >&2
  if ask_yes_no "Use the local MariaDB (in-stack)?" "y"; then
    MARIADB_EXTERNAL=false
    case "$TARGET" in
      compose)   MARIADB_HOST=mariadb ;;
      k8s-local) MARIADB_HOST=mariadb.insight-infra.svc.cluster.local ;;
    esac
    MARIADB_PORT=3306
    MARIADB_USER=insight
    MARIADB_PASSWORD=insight-local
    MARIADB_ROOT_PASSWORD=root-local
  else
    MARIADB_EXTERNAL=true
    MARIADB_HOST=$(ask "  External MariaDB host" "")
    [[ -z "$MARIADB_HOST" ]] && { echo "  ERROR: host is required." >&2; exit 1; }
    [[ "$TARGET" == "compose" ]] && warn_localhost_host "$MARIADB_HOST" "MariaDB"
    MARIADB_PORT=$(ask "  External MariaDB port" "3306")
    MARIADB_USER=$(ask "  MariaDB user" "insight")
    MARIADB_PASSWORD=$(ask_secret "  MariaDB password")
    # For compose, the root password is unused once MARIADB_EXTERNAL=true
    # (nothing in the stack authenticates as root against an external DB).
    # For k8s-local, the wizard writes the root password into the
    # insight-db-creds Secret which the chart-side init job consumes —
    # so we must collect the real value, not a placeholder.
    if [[ "$TARGET" == "k8s-local" ]]; then
      MARIADB_ROOT_PASSWORD=$(ask_secret "  MariaDB root password (used by cluster init jobs)")
      [[ -z "$MARIADB_ROOT_PASSWORD" ]] && { echo "  ERROR: root password is required for k8s-local external MariaDB." >&2; exit 1; }
    else
      MARIADB_ROOT_PASSWORD=root-local
    fi
    # Connectivity probe — same for both targets, requires docker.
    if command -v docker >/dev/null 2>&1; then
      validate_mariadb "$MARIADB_HOST" "$MARIADB_PORT" "$MARIADB_USER" "$MARIADB_PASSWORD" || exit 1
    else
      echo "  (skipping connectivity probe — docker not available)" >&2
    fi
  fi
  echo "" >&2

  # ── ClickHouse ────────────────────────────────────────────────────
  echo "--- ClickHouse ---" >&2
  if ask_yes_no "Use the local ClickHouse (in-stack)?" "y"; then
    CLICKHOUSE_EXTERNAL=false
    case "$TARGET" in
      compose)   CLICKHOUSE_HOST=clickhouse ;;
      k8s-local) CLICKHOUSE_HOST=clickhouse.insight-infra.svc.cluster.local ;;
    esac
    CLICKHOUSE_HTTP_PORT=8123
    CLICKHOUSE_DATABASE=insight
    CLICKHOUSE_USER=insight
    CLICKHOUSE_PASSWORD=insight-local
  else
    CLICKHOUSE_EXTERNAL=true
    CLICKHOUSE_HOST=$(ask "  External ClickHouse host" "")
    [[ -z "$CLICKHOUSE_HOST" ]] && { echo "  ERROR: host is required." >&2; exit 1; }
    [[ "$TARGET" == "compose" ]] && warn_localhost_host "$CLICKHOUSE_HOST" "ClickHouse"
    CLICKHOUSE_HTTP_PORT=$(ask "  External ClickHouse HTTP port" "8123")
    CLICKHOUSE_DATABASE=$(ask   "  ClickHouse database" "insight")
    CLICKHOUSE_USER=$(ask       "  ClickHouse user" "insight")
    CLICKHOUSE_PASSWORD=$(ask_secret "  ClickHouse password")
    if command -v curl >/dev/null 2>&1; then
      validate_clickhouse "$CLICKHOUSE_HOST" "$CLICKHOUSE_HTTP_PORT" "$CLICKHOUSE_USER" "$CLICKHOUSE_PASSWORD" "$CLICKHOUSE_DATABASE" || exit 1
    else
      echo "  (skipping connectivity probe — curl not available)" >&2
    fi
  fi
  echo "" >&2

  # ── Tenant ID ─────────────────────────────────────────────────────
  if [[ "$MARIADB_EXTERNAL" == "true" || "$CLICKHOUSE_EXTERNAL" == "true" ]]; then
    echo "--- Tenant ID ---" >&2
    echo "  External DBs already contain data tied to a specific tenant." >&2
    echo "  Enter the UUID present in persons.insight_tenant_id." >&2
    TENANT_DEFAULT_ID=$(ask "  TENANT_DEFAULT_ID" "")
    if [[ -z "$TENANT_DEFAULT_ID" ]]; then
      echo "  ERROR: tenant ID is required when using external DBs." >&2
      exit 1
    fi
    echo "" >&2
  else
    TENANT_DEFAULT_ID="$TENANT_DEFAULT_UUID"
  fi

  # ── Dev impersonation email ───────────────────────────────────────
  echo "--- Dev impersonation ---" >&2
  DEV_USER_EMAIL=$(ask "VITE_DEV_USER_EMAIL" "dev@company.nonpresent")
  echo "" >&2
}

# ──────────────────────────────────────────────────────────────────────
# Compose target
# ──────────────────────────────────────────────────────────────────────

write_compose() {
  local env_file="$ROOT_DIR/.env.compose"
  local example="$ROOT_DIR/.env.compose.example"

  if [[ -e "$env_file" ]]; then
    echo "ERROR: $env_file already exists — delete it first to re-run the wizard." >&2
    exit 1
  fi
  if [[ ! -f "$example" ]]; then
    echo "ERROR: $example is missing — can't bootstrap .env.compose." >&2
    exit 1
  fi

  preflight_compose
  ask_shared

  # ── Frontend mode (compose-only) ──────────────────────────────────
  echo "--- Frontend ---" >&2
  local fe_mode fe_path default_fe_path="../insight-front"
  echo "  How should the frontend run?" >&2
  echo "    1) ghcr   — pull the pre-built image (no source needed)" >&2
  echo "    2) local  — Vite + HMR against an existing insight-front checkout" >&2
  echo "    3) clone  — git clone insight-front, then run Vite + HMR" >&2
  local fe_choice
  while true; do
    fe_choice=$(ask "  Choice" "1")
    case "$fe_choice" in
      1|ghcr)
        fe_mode="ghcr"
        fe_path="$default_fe_path"
        break ;;
      2|local|dev)
        fe_mode="dev"
        fe_path=$(ask "  Path to insight-front checkout" "$default_fe_path")
        if [[ -z "$fe_path" || ! -d "$ROOT_DIR/$fe_path" && ! -d "$fe_path" ]]; then
          echo "  ERROR: '$fe_path' does not exist. Pick option 3 to clone." >&2
          exit 1
        fi
        break ;;
      3|clone)
        if ! command -v git >/dev/null 2>&1; then
          echo "  ERROR: git is not installed; pick 1 or 2." >&2
          continue
        fi
        fe_path=$(ask "  Clone insight-front into" "$default_fe_path")
        # Resolve relative paths against the repo root.
        local clone_target
        if [[ "$fe_path" = /* ]]; then clone_target="$fe_path"
        else clone_target="$ROOT_DIR/$fe_path"; fi
        if [[ -e "$clone_target" ]]; then
          echo "  ERROR: '$clone_target' already exists; refusing to clone over it." >&2
          echo "         Remove it first, or pick 2 to reuse the existing checkout." >&2
          exit 1
        fi
        if ! git clone https://github.com/constructorfabric/insight-front.git "$clone_target" >&2; then
          echo "  ERROR: clone failed." >&2
          exit 1
        fi
        fe_mode="dev"
        break ;;
      *) echo "  Please answer 1, 2, or 3." >&2 ;;
    esac
  done
  echo "" >&2

  # ── Auth mode (compose-only) ──────────────────────────────────────
  echo "--- Auth ---" >&2
  local auth_mode
  echo "  Which auth backend should the authenticator log in against?" >&2
  echo "    1) fakeidp   — no login screen, binds to VITE_DEV_USER_EMAIL (default)" >&2
  echo "    2) keycloak  — real Keycloak login form + custom claims (:8085)" >&2
  local auth_choice
  while true; do
    auth_choice=$(ask "  Choice" "1")
    case "$auth_choice" in
      1|fakeidp)
        auth_mode="fakeidp"
        break ;;
      2|keycloak)
        auth_mode="keycloak"
        break ;;
      *) echo "  Please answer 1 or 2." >&2 ;;
    esac
  done
  echo "" >&2

  # ── Seeding decision for external DBs ─────────────────────────────
  local seed_external=false
  if [[ "$MARIADB_EXTERNAL" == "true" || "$CLICKHOUSE_EXTERNAL" == "true" ]]; then
    echo "--- Test data ---" >&2
    echo "  Local DBs are always seeded on first up. For external DBs the" >&2
    echo "  wizard leaves them alone unless you opt in here." >&2
    if ask_yes_no "  Seed test data into your external DB(s)?" "n"; then
      seed_external=true
    fi
    echo "" >&2
  fi

  # ── Write .env.compose ────────────────────────────────────────────
  cp "$example" "$env_file"
  update_env_var "$env_file" MARIADB_EXTERNAL              "$MARIADB_EXTERNAL"
  update_env_var "$env_file" MARIADB_HOST                  "$MARIADB_HOST"
  update_env_var "$env_file" MARIADB_INTERNAL_PORT         "$MARIADB_PORT"
  update_env_var "$env_file" MARIADB_USER                  "$MARIADB_USER"
  update_env_var "$env_file" MARIADB_PASSWORD              "$MARIADB_PASSWORD"
  update_env_var "$env_file" MARIADB_ROOT_PASSWORD         "$MARIADB_ROOT_PASSWORD"
  update_env_var "$env_file" CLICKHOUSE_EXTERNAL           "$CLICKHOUSE_EXTERNAL"
  update_env_var "$env_file" CLICKHOUSE_HOST               "$CLICKHOUSE_HOST"
  update_env_var "$env_file" CLICKHOUSE_INTERNAL_HTTP_PORT "$CLICKHOUSE_HTTP_PORT"
  update_env_var "$env_file" CLICKHOUSE_DATABASE           "$CLICKHOUSE_DATABASE"
  update_env_var "$env_file" CLICKHOUSE_USER               "$CLICKHOUSE_USER"
  update_env_var "$env_file" CLICKHOUSE_PASSWORD           "$CLICKHOUSE_PASSWORD"
  update_env_var "$env_file" TENANT_DEFAULT_ID             "$TENANT_DEFAULT_ID"
  update_env_var "$env_file" VITE_DEV_USER_EMAIL           "$DEV_USER_EMAIL"
  update_env_var "$env_file" FRONTEND_MODE                 "$fe_mode"
  update_env_var "$env_file" INSIGHT_FRONT_PATH            "$fe_path"
  update_env_var "$env_file" AUTH_MODE                     "$auth_mode"

  # SEEDED_LOCAL_* gates the first-run auto-seed in dev-compose.sh.
  if [[ "$MARIADB_EXTERNAL" == "true" && "$seed_external" != "true" ]]; then
    update_env_var "$env_file" SEEDED_LOCAL_MARIA true
  fi
  if [[ "$CLICKHOUSE_EXTERNAL" == "true" && "$seed_external" != "true" ]]; then
    update_env_var "$env_file" SEEDED_LOCAL_CH true
  fi

  echo "Wrote $env_file." >&2
}

# ──────────────────────────────────────────────────────────────────────
# K8s-local target — writes the gitops env files for `ENV=local`.
#
# Outputs (under deploy/gitops/):
#   environments/local/inventory.yaml   (concrete, gitignored)
#   environments/local/.env.local       (chain-sourced env vars: airbyte
#                                        setup creds. gitignored.)
#   environments/local/values.yaml      (concrete umbrella overlay,
#                                        cp'd from values.yaml.template
#                                        then mutated with the wizard's
#                                        tenant ID. gitignored.)
#   secrets-store.yaml                  (cleartext, gitignored — read by
#                                        scripts/secret-fetch.sh during seal)
#
# Does NOT provision the cluster, fetch the kubeseal pub-cert, or run
# `helm install`. Those happen in subsequent Makefile chain steps
# (bootstrap → fetch-cert → seal → system → deploy).
# ──────────────────────────────────────────────────────────────────────

# Auto-detect cluster type from kube-context name, returns one of
# kind | k3d | k3s | orbstack | colima | minikube | remote (best-effort).
detect_cluster_type() {
  local ctx="$1"
  case "$ctx" in
    kind-*)         echo "kind" ;;
    k3d-*)          echo "k3d" ;;
    orbstack|orbstack-*) echo "orbstack" ;;
    colima|colima-*)     echo "colima" ;;
    minikube|*minikube*) echo "minikube" ;;
    *k3s*|insight-local) echo "k3s" ;;
    *)              echo "remote" ;;
  esac
}

write_k8s_local() {
  local gitops_dir="$ROOT_DIR/deploy/gitops"
  local inventory_out="$gitops_dir/environments/local/inventory.yaml"
  local inventory_tmpl="$gitops_dir/environments/local/inventory.yaml.template"
  local values_out="$gitops_dir/environments/local/values.yaml"
  local values_tmpl="$gitops_dir/environments/local/values.yaml.template"
  local env_local_out="$gitops_dir/environments/local/.env.local"
  local secrets_store_out="$gitops_dir/secrets-store.yaml"
  local secrets_store_tmpl="$gitops_dir/secrets-store.yaml.template"

  for f in "$inventory_out" "$values_out" "$secrets_store_out"; do
    if [[ -e "$f" ]]; then
      echo "ERROR: $f already exists — delete it first to re-run the wizard." >&2
      [[ "$f" == "$secrets_store_out" ]] && echo "       (Contains cleartext passwords; verify you're not overwriting real state.)" >&2
      exit 1
    fi
  done
  for f in "$inventory_tmpl" "$values_tmpl" "$secrets_store_tmpl"; do
    if [[ ! -f "$f" ]]; then
      echo "ERROR: $f is missing — can't bootstrap from template." >&2
      exit 1
    fi
  done

  preflight_k8s
  ask_shared

  # ── Kube-context ─────────────────────────────────────────────────
  #
  # kubectl/helm/kubeseal read from $KUBECONFIG (or ~/.kube/config if
  # unset). If the cluster you want isn't listed, abort, re-run with the
  # right kubeconfig:
  #     KUBECONFIG=/path/to/config.yaml make deploy ENV=local
  echo "--- Kubernetes cluster ---" >&2
  if [[ -n "${KUBECONFIG:-}" ]]; then
    echo "  Using KUBECONFIG=$KUBECONFIG" >&2
  else
    echo "  Using ~/.kube/config (KUBECONFIG not set)" >&2
  fi
  local available current default_ctx kube_ctx
  available=$(kubectl config get-contexts -o name 2>/dev/null || true)
  current=$(kubectl config current-context 2>/dev/null || true)
  default_ctx="${current:-insight-local}"
  if [[ -z "$available" ]]; then
    echo "  No kube-contexts found in your kubeconfig." >&2
    echo "  Either provision a cluster (kind / k3d / OrbStack / …) and re-run," >&2
    echo "  or point at a different kubeconfig:" >&2
    echo "    KUBECONFIG=/path/to/config.yaml make deploy ENV=local" >&2
    exit 1
  fi
  echo "  Available contexts:" >&2
  printf '%s\n' "$available" | while IFS= read -r ctx; do
    [[ -n "$ctx" ]] && printf '    - %s\n' "$ctx" >&2
  done
  kube_ctx=$(ask "  Kube context" "$default_ctx")
  if ! printf '%s\n' "$available" | grep -qFx -- "$kube_ctx"; then
    echo "  ERROR: '$kube_ctx' is not in your kubeconfig." >&2
    exit 1
  fi
  echo "  Probing cluster '${kube_ctx}'..." >&2
  if ! kubectl --context "$kube_ctx" --request-timeout=5s cluster-info >/dev/null 2>&1; then
    echo "  ERROR: cannot reach cluster '$kube_ctx'." >&2
    echo "         Bring it up (kind create cluster / k3d cluster create / OrbStack …)" >&2
    echo "         and re-run. The wizard does not provision clusters." >&2
    exit 1
  fi
  local cluster_type
  cluster_type=$(detect_cluster_type "$kube_ctx")
  echo "  OK (cluster type: $cluster_type)" >&2
  echo "" >&2

  # ── L0 cluster prereqs pre-flight ────────────────────────────────
  #
  # bootstrap-* targets call `helm upgrade --install`, which fails if a
  # matching resource already exists but isn't Helm-managed (OrbStack
  # ships its own ingress-nginx, k3s sometimes ships traefik+klipper, a
  # shared sandbox cluster might have cert-manager from another stack,
  # etc). For each controller, probe the cluster, show what's there,
  # and let the operator decide whether to skip our install. No silent
  # reconfig — if the operator wants to install anyway and it fails,
  # the Makefile aborts with helm's own error and they can clean up.
  echo "--- L0 cluster prereqs (preflight) ---" >&2
  local l0_ingress_nginx=true l0_cert_manager=true l0_sealed_secrets=true
  _check_l0_controller() {
    local label="$1" ns="$2" release="$3" probe_kind="$4" probe_name="$5"
    local found_present=false found_helm=false
    if kubectl --context "$kube_ctx" get namespace "$ns" >/dev/null 2>&1; then
      if kubectl --context "$kube_ctx" -n "$ns" get "$probe_kind" "$probe_name" >/dev/null 2>&1; then
        found_present=true
        if helm --kube-context "$kube_ctx" list -n "$ns" -q 2>/dev/null | grep -qFx -- "$release"; then
          found_helm=true
        fi
      fi
    fi
    if [[ "$found_present" == "false" ]]; then
      echo "  $label: not installed → bootstrap will install." >&2
      return 0
    fi
    if [[ "$found_helm" == "true" ]]; then
      echo "  $label: already Helm-managed (release '$release' in '$ns') → bootstrap will upgrade in place." >&2
      return 0
    fi
    echo "  $label: $probe_kind '$probe_name' exists in '$ns' but isn't Helm-managed." >&2
    echo "    Installing via bootstrap will fail (ownership conflict)." >&2
    if ask_yes_no "    Skip installing $label via bootstrap?" "y"; then
      return 1
    fi
    echo "    OK — bootstrap will proceed; clean up the existing install manually if helm refuses." >&2
    return 0
  }
  _check_l0_controller "ingress-nginx"      ingress-nginx ingress-nginx            sa     ingress-nginx           || l0_ingress_nginx=false
  _check_l0_controller "cert-manager"       cert-manager  cert-manager             deploy cert-manager            || l0_cert_manager=false
  _check_l0_controller "sealed-secrets"     kube-system   sealed-secrets-controller deploy sealed-secrets-controller || l0_sealed_secrets=false
  echo "" >&2

  # ── L2 services to install ───────────────────────────────────────
  echo "--- L2 services ---" >&2
  echo "  MariaDB / ClickHouse / Redis / Redpanda are required (always on)." >&2
  echo "  Pick the optional services to install:" >&2
  local sys_airbyte sys_argo sys_redpanda_console sys_obs airbyte_email airbyte_org
  if ask_yes_no "  Install Airbyte (data ingestion)?" "y"; then
    sys_airbyte=true
    airbyte_email=$(ask  "    Airbyte setup admin email" "admin@example.com")
    airbyte_org=$(ask    "    Airbyte setup workspace name" "Insight")
  else
    sys_airbyte=false
  fi
  if ask_yes_no "  Install Argo Workflows (job orchestration)?" "y"; then
    sys_argo=true
  else
    sys_argo=false
  fi
  if ask_yes_no "  Install redpanda-console (Kafka UI)?" "n"; then
    sys_redpanda_console=true
  else
    sys_redpanda_console=false
  fi
  if ask_yes_no "  Install observability stack (Loki + Alloy + Grafana)?" "n"; then
    sys_obs=true
  else
    sys_obs=false
  fi
  echo "" >&2

  # ── Write inventory.yaml ─────────────────────────────────────────
  cp "$inventory_tmpl" "$inventory_out"
  yq -i ".kubeContext = \"$kube_ctx\"" "$inventory_out"
  yq -i ".bootstrap.ingressNginx  = $l0_ingress_nginx"  "$inventory_out"
  yq -i ".bootstrap.certManager   = $l0_cert_manager"   "$inventory_out"
  yq -i ".bootstrap.sealedSecrets = $l0_sealed_secrets" "$inventory_out"
  yq -i ".system.airbyte         = $sys_airbyte"          "$inventory_out"
  yq -i ".system.argoWorkflows   = $sys_argo"             "$inventory_out"
  yq -i ".system.redpandaConsole = $sys_redpanda_console" "$inventory_out"
  yq -i ".system.loki    = $sys_obs" "$inventory_out"
  yq -i ".system.alloy   = $sys_obs" "$inventory_out"
  yq -i ".system.grafana = $sys_obs" "$inventory_out"
  echo "Wrote $inventory_out." >&2

  # ── Write .env.local (airbyte chain creds) ───────────────────────
  # The chain sources this file via `set -a; . ./.env.local; set +a`, so
  # the values must be safe to re-parse by a sourced bash. `printf %q`
  # quotes whitespace + shell metacharacters; even though the wizard's
  # defaults are tame, an org name like "Acme & Co" or an email with a
  # `$` would otherwise corrupt the sourced env.
  if [[ "$sys_airbyte" == "true" ]]; then
    {
      echo "# Generated by deploy/compose/insight-init.sh on first \`make deploy ENV=local\`."
      echo "# Sourced by the local-chain target before invoking \`make system-airbyte\`."
      echo "# Gitignored; safe to delete and regenerate."
      printf 'AIRBYTE_SETUP_EMAIL=%q\n' "$airbyte_email"
      printf 'AIRBYTE_SETUP_ORG=%q\n'   "$airbyte_org"
    } > "$env_local_out"
    echo "Wrote $env_local_out." >&2
  fi

  # ── Write secrets-store.yaml ─────────────────────────────────────
  # The template has 5 top-level keys: insight-local-{mariadb,clickhouse,redis}-creds
  # under insight-infra and insight-local-insight-{db-creds,oidc} under insight.
  # The wizard fills the cleartext from collected passwords; the oidc entry
  # is left commented (sandbox runs with authDisabled=true).
  #
  # YAML-escape passwords before interpolating — operator-supplied external
  # DB passwords can contain `"` or `\`, which would otherwise produce
  # invalid manifests and break the seal step. Defaults (insight-local /
  # root-local) are safe but we don't want the path to diverge per input.
  yaml_escape() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }
  local mdb_root_esc mdb_pass_esc ch_pass_esc
  mdb_root_esc=$(yaml_escape "$MARIADB_ROOT_PASSWORD")
  mdb_pass_esc=$(yaml_escape "$MARIADB_PASSWORD")
  ch_pass_esc=$(yaml_escape  "$CLICKHOUSE_PASSWORD")
  cat > "$secrets_store_out" <<EOF
##
## secrets-store.yaml — local cleartext store consumed by
## scripts/secret-fetch.sh. Generated by deploy/compose/insight-init.sh.
## Gitignored — NEVER COMMIT.
##
## Top-level keys are resource names (insight-local-<secret-base>).
## Values are cleartext Kubernetes Secret manifests; make seal-secret
## pipes them through kubeseal into committable sealed manifests.
##

insight-local-mariadb-creds: |
  apiVersion: v1
  kind: Secret
  metadata:
    name: mariadb-creds
    namespace: insight-infra
  type: Opaque
  stringData:
    mariadb-root-password: "$mdb_root_esc"
    mariadb-password:      "$mdb_pass_esc"

insight-local-clickhouse-creds: |
  apiVersion: v1
  kind: Secret
  metadata:
    name: clickhouse-creds
    namespace: insight-infra
  type: Opaque
  stringData:
    admin-password: "$ch_pass_esc"

insight-local-redis-creds: |
  apiVersion: v1
  kind: Secret
  metadata:
    name: redis-creds
    namespace: insight-infra
  type: Opaque
  stringData:
    redis-password: "redis-local"

insight-local-insight-db-creds: |
  apiVersion: v1
  kind: Secret
  metadata:
    name: insight-db-creds
    namespace: insight
  type: Opaque
  stringData:
    mariadb-root-password: "$mdb_root_esc"
    mariadb-password:      "$mdb_pass_esc"
    clickhouse-password:   "$ch_pass_esc"
    redis-password:        "redis-local"

## insight-oidc — required only when authDisabled=false. The shipped
## local overlay disables auth, so this is left commented. Uncomment +
## fill in for envs with a real IdP, then add insight-oidc to
## inventory.secrets.services.
# insight-local-insight-oidc: |
#   apiVersion: v1
#   kind: Secret
#   metadata:
#     name: insight-oidc
#     namespace: insight
#   type: Opaque
#   stringData:
#     APP__gears__oidc-authn-plugin__config__issuer_url: "https://<idp>/..."
#     APP__gears__oidc-authn-plugin__config__audience:   "<api-audience>"
#     APP__gears__oidc-authn-plugin__config__jwks_url:   "https://<idp>/.../jwks.json"
#     APP__gears__auth-info__config__issuer_url:         "https://<idp>/..."
#     APP__gears__auth-info__config__client_id:          "<client-id>"
#     APP__gears__auth-info__config__redirect_uri:       "https://<host>/callback"
#     APP__gears__auth-info__config__scopes:             "openid profile email"
EOF
  echo "Wrote $secrets_store_out." >&2
  echo "" >&2

  # ── Write values.yaml from template ──────────────────────────────
  # cp values.yaml.template → values.yaml, then inject the collected
  # tenant id under .global.tenantDefaultId and the dev impersonation
  # email under .frontend.devUserEmail. The live values.yaml is
  # gitignored — the wizard regenerates it per-developer; the template
  # holds the committed sandbox config.
  cp "$values_tmpl" "$values_out"
  yq -i ".global.tenantDefaultId = \"$TENANT_DEFAULT_ID\"" "$values_out"
  yq -i ".frontend.devUserEmail  = \"$DEV_USER_EMAIL\""    "$values_out"
  echo "Wrote $values_out." >&2

  cat >&2 <<EOF

Next: \`make deploy ENV=local\` (already running, if invoked from there)
will continue with: bootstrap → fetch-cert → seal → system → deploy-app.

Manual demo-data seeding (the compose stack auto-seeds; the k8s stack
doesn't ship a seed image yet — port-forward and run from the host):

  kubectl -n insight-infra port-forward svc/mariadb    3306:3306 &
  kubectl -n insight-infra port-forward svc/clickhouse 8123:8123 &
  cd $ROOT_DIR/deploy/seed
  python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
  MARIADB_HOST=127.0.0.1 CLICKHOUSE_HOST=127.0.0.1 \\
    MARIADB_USER=$MARIADB_USER MARIADB_PASSWORD=$MARIADB_PASSWORD \\
    CLICKHOUSE_USER=$CLICKHOUSE_USER CLICKHOUSE_PASSWORD=$CLICKHOUSE_PASSWORD \\
    .venv/bin/python seed.py all

See deploy/seed/README.md for the package layout.

EOF
}

# ──────────────────────────────────────────────────────────────────────
# Dispatch
# ──────────────────────────────────────────────────────────────────────

case "$TARGET" in
  compose)   write_compose ;;
  k8s-local) write_k8s_local ;;
esac

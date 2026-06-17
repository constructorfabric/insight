#!/usr/bin/env bash
# Insight platform — docker-compose dev stack control surface.
#
# Subcommands:
#   up       Bring the stack up. On first run it walks you through
#            generating .env.compose, then builds artefacts, generates
#            the per-run compose override, starts every service per
#            the chosen profile, and seeds demo data into any local DB.
#   down     Stop everything (data preserved by default).
#   build    Rebuild one service's host-side artefact.
#   seed     Populate the demo dataset (identity / silver / all).
#   prune    Destructive wipe — containers, volumes, build/, override,
#            and .env.compose. Always interactive.
#   help     Print this message.
#
# Each subcommand has its own --help.
#
# Most settings live in .env.compose. See .env.compose.example for the
# full contract and CONTRIBUTING.md for the daily workflow.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# ──────────────────────────────────────────────────────────────────────
# Shared helpers
# ──────────────────────────────────────────────────────────────────────

# bash 3.2 (Mac default) lacks associative arrays. Plain strings + tiny
# helpers keep this script portable.
trim()     { local s="$1"; s="${s#"${s%%[![:space:]]*}"}"; s="${s%"${s##*[![:space:]]}"}"; printf '%s' "$s"; }
contains() { case " $1 " in *" $2 "*) return 0 ;; esac; return 1; }
add()      { local list="$1" item="$2"; contains "$list" "$item" && printf '%s' "$list" || printf '%s %s' "$list" "$item"; }

resolve_env_file() {
  local f="${1:-.env.compose}"
  [[ -f "$f" ]] && { printf '%s' "$f"; return 0; }
  [[ "$f" == ".env.compose" && -f ".env.compose.example" ]] && {
    printf '%s' ".env.compose.example"
    return 0
  }
  echo "ERROR: env file not found: $f" >&2
  echo "       Run:  ./dev-compose.sh up   (the first-run wizard will" >&2
  echo "       create .env.compose), or copy .env.compose.example manually." >&2
  return 1
}

# ──────────────────────────────────────────────────────────────────────
# First-run wizard helpers
#
# Used by cmd_up to generate .env.compose interactively when it's
# missing. Everything here is io-helpers and validation — the wizard
# proper is `bootstrap_env_wizard` below.
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

# ask_secret <prompt> — read a password without echoing it. No default
# (passwords shouldn't have defaults you can't see).
ask_secret() {
  local prompt="$1" answer
  printf '%s: ' "$prompt" >&2
  read -rs answer
  printf '\n' >&2
  printf '%s' "$answer"
}

# ask_yes_no <prompt> <default y|n> — loops until a yes/no answer; return
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
# and GNU sed by writing through a temp file.
update_env_var() {
  local file="$1" key="$2" value="$3" escaped tmp
  # Escape sed replacement-side metacharacters: \, &, and our delimiter |.
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
# inside the container, that points at the container itself. Common
# gotcha; the wizard can't fix it but should flag it.
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

# validate_mariadb host port user pass — runs `mariadb -e "SELECT 1"`
# inside a transient mariadb container. Returns 0 on success.
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

# validate_clickhouse host http_port user pass db — issues SELECT 1 via
# the HTTP interface using host-side curl. Returns 0 on success.
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

# Walk the user through the questions needed to populate .env.compose
# the first time. Caller (cmd_up) has already verified that
# .env.compose is missing AND the env-file path wasn't overridden.
bootstrap_env_wizard() {
  if [[ ! -t 0 ]]; then
    echo "ERROR: .env.compose is missing and stdin is not a TTY." >&2
    echo "       The first-run wizard needs an interactive shell." >&2
    echo "       Copy .env.compose.example to .env.compose and edit," >&2
    echo "       or re-run ./dev-compose.sh up from a terminal." >&2
    return 1
  fi
  if [[ ! -f .env.compose.example ]]; then
    echo "ERROR: .env.compose.example is missing — can't bootstrap." >&2
    return 1
  fi

  cat >&2 <<'EOF'

=== First-run wizard: configuring .env.compose ===

You're bringing up the Insight dev stack for the first time. This
wizard collects the few values needed to generate .env.compose, then
hands off to `docker compose up`. Press Enter to accept defaults shown
in [brackets].

EOF

  # ── MariaDB ─────────────────────────────────────────────────────────
  local mariadb_external maria_host maria_port maria_user maria_pass
  local maria_root_pass=root-local
  echo "--- MariaDB ---" >&2
  if ask_yes_no "Use the local MariaDB in docker compose?" "y"; then
    mariadb_external=false
    maria_host=mariadb
    maria_port=3306
    maria_user=insight
    maria_pass=insight-local
  else
    mariadb_external=true
    maria_host=$(ask "  External MariaDB host" "")
    [[ -z "$maria_host" ]] && { echo "  ERROR: host is required." >&2; return 1; }
    warn_localhost_host "$maria_host" "MariaDB"
    maria_port=$(ask "  External MariaDB port" "3306")
    maria_user=$(ask "  MariaDB user" "insight")
    maria_pass=$(ask_secret "  MariaDB password")
    validate_mariadb "$maria_host" "$maria_port" "$maria_user" "$maria_pass" || return 1
  fi
  echo "" >&2

  # ── ClickHouse ──────────────────────────────────────────────────────
  local ch_external ch_host ch_port ch_db ch_user ch_pass
  echo "--- ClickHouse ---" >&2
  if ask_yes_no "Use the local ClickHouse in docker compose?" "y"; then
    ch_external=false
    ch_host=clickhouse
    ch_port=8123
    ch_db=insight
    ch_user=insight
    ch_pass=insight-local
  else
    ch_external=true
    ch_host=$(ask "  External ClickHouse host" "")
    [[ -z "$ch_host" ]] && { echo "  ERROR: host is required." >&2; return 1; }
    warn_localhost_host "$ch_host" "ClickHouse"
    ch_port=$(ask "  External ClickHouse HTTP port" "8123")
    ch_db=$(ask   "  ClickHouse database" "insight")
    ch_user=$(ask "  ClickHouse user" "insight")
    ch_pass=$(ask_secret "  ClickHouse password")
    validate_clickhouse "$ch_host" "$ch_port" "$ch_user" "$ch_pass" "$ch_db" || return 1
  fi
  echo "" >&2

  # ── Tenant ID (only meaningful when binding to existing data) ───────
  local tenant_id
  if [[ "$mariadb_external" == "true" || "$ch_external" == "true" ]]; then
    echo "--- Tenant ID ---" >&2
    echo "  External DBs already contain data tied to a specific tenant." >&2
    echo "  Enter the UUID present in persons.insight_tenant_id." >&2
    tenant_id=$(ask "  TENANT_DEFAULT_ID" "")
    if [[ -z "$tenant_id" ]]; then
      echo "  ERROR: tenant ID is required when using external DBs." >&2
      return 1
    fi
    echo "" >&2
  else
    tenant_id="00000000-df51-5b42-9538-d2b56b7ee953"
  fi

  # ── Email ───────────────────────────────────────────────────────────
  echo "--- Dev impersonation ---" >&2
  local dev_email
  dev_email=$(ask "VITE_DEV_USER_EMAIL" "dev@company.nonpresent")
  echo "" >&2

  # ── Frontend ────────────────────────────────────────────────────────
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
        if [[ -z "$fe_path" || ! -d "$fe_path" ]]; then
          echo "  ERROR: '$fe_path' does not exist. Pick option 3 to clone." >&2
          return 1
        fi
        break ;;
      3|clone)
        if ! command -v git >/dev/null 2>&1; then
          echo "  ERROR: git is not installed; pick 1 or 2." >&2
          continue
        fi
        fe_path=$(ask "  Clone insight-front into" "$default_fe_path")
        if [[ -e "$fe_path" ]]; then
          echo "  ERROR: '$fe_path' already exists; refusing to clone over it." >&2
          echo "         Remove it first, or pick 2 to reuse the existing checkout." >&2
          return 1
        fi
        if ! git clone https://github.com/constructorfabric/insight-front.git "$fe_path" >&2; then
          echo "  ERROR: clone failed." >&2
          return 1
        fi
        fe_mode="dev"
        break ;;
      *)
        echo "  Please answer 1, 2, or 3." >&2 ;;
    esac
  done
  echo "" >&2

  # ── Seeding decision for external DBs ───────────────────────────────
  local seed_external=false
  if [[ "$mariadb_external" == "true" || "$ch_external" == "true" ]]; then
    echo "--- Test data ---" >&2
    echo "  Local DBs are always seeded on first up. For external DBs the" >&2
    echo "  wizard leaves them alone unless you opt in here." >&2
    if ask_yes_no "  Seed test data into your external DB(s)?" "n"; then
      seed_external=true
    fi
    echo "" >&2
  fi

  # ── Write .env.compose ──────────────────────────────────────────────
  cp .env.compose.example .env.compose
  update_env_var .env.compose MARIADB_EXTERNAL          "$mariadb_external"
  update_env_var .env.compose MARIADB_HOST              "$maria_host"
  update_env_var .env.compose MARIADB_INTERNAL_PORT     "$maria_port"
  update_env_var .env.compose MARIADB_USER              "$maria_user"
  update_env_var .env.compose MARIADB_PASSWORD          "$maria_pass"
  update_env_var .env.compose MARIADB_ROOT_PASSWORD     "$maria_root_pass"
  update_env_var .env.compose CLICKHOUSE_EXTERNAL       "$ch_external"
  update_env_var .env.compose CLICKHOUSE_HOST           "$ch_host"
  update_env_var .env.compose CLICKHOUSE_INTERNAL_HTTP_PORT "$ch_port"
  update_env_var .env.compose CLICKHOUSE_DATABASE       "$ch_db"
  update_env_var .env.compose CLICKHOUSE_USER           "$ch_user"
  update_env_var .env.compose CLICKHOUSE_PASSWORD       "$ch_pass"
  update_env_var .env.compose TENANT_DEFAULT_ID         "$tenant_id"
  update_env_var .env.compose VITE_DEV_USER_EMAIL       "$dev_email"
  update_env_var .env.compose FRONTEND_MODE             "$fe_mode"
  update_env_var .env.compose INSIGHT_FRONT_PATH        "$fe_path"

  # SEEDED_LOCAL_* gates the first-run auto-seed in cmd_up.
  #   - empty/false → seed will run.
  #   - true        → seed will be skipped.
  # For external DBs the user explicitly declined, we pre-mark them
  # seeded so cmd_up doesn't touch them.
  if [[ "$mariadb_external" == "true" && "$seed_external" != "true" ]]; then
    update_env_var .env.compose SEEDED_LOCAL_MARIA true
  fi
  if [[ "$ch_external" == "true" && "$seed_external" != "true" ]]; then
    update_env_var .env.compose SEEDED_LOCAL_CH true
  fi

  echo "Wrote .env.compose. Continuing with up..." >&2
  echo "" >&2
}

# ──────────────────────────────────────────────────────────────────────
# up
# ──────────────────────────────────────────────────────────────────────

cmd_up_help() {
  cat <<'EOF'
usage: dev-compose.sh up [options]

Bring the stack up: build host-side artefacts (Rust + .NET + optional
frontend dist), generate a per-run compose override that flips selected
services to ghcr images, then `docker compose up -d`.

Options:
  --from-ghcr=svc1,svc2     Pull these backend services from ghcr instead
                            of building. Recognised: api-gateway,
                            analytics-api, identity.
  --build-only=svc1,svc2    Build only these; everything else from ghcr.
  --frontend-mode=MODE      Override FRONTEND_MODE for this run.
                            (dev | built | ghcr)
  --no-frontend             Don't start any frontend variant.
  --skip-build              Don't rebuild artefacts — reuse what's
                            already in compose/build/.
  --env-file=PATH           Alternate dotenv file. Default: .env.compose.

Out-of-scope:
  --start-airbyte / --start-argo
      Both need k8s and are not shipped by this compose stack.
      The script exits with a pointer to CONTRIBUTING.md if you pass them.
EOF
}

cmd_up() {
  local env_file=".env.compose"
  local from_ghcr_csv=""
  local build_only_csv=""
  local frontend_mode_override=""
  local skip_build=false
  local no_frontend=false

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*)      env_file="${1#*=}"; shift ;;
      --env-file)        env_file="$2"; shift 2 ;;
      --from-ghcr=*)     from_ghcr_csv="${1#*=}"; shift ;;
      --from-ghcr)       from_ghcr_csv="$2"; shift 2 ;;
      --build-only=*)    build_only_csv="${1#*=}"; shift ;;
      --build-only)      build_only_csv="$2"; shift 2 ;;
      --frontend-mode=*) frontend_mode_override="${1#*=}"; shift ;;
      --frontend-mode)   frontend_mode_override="$2"; shift 2 ;;
      --skip-build)      skip_build=true; shift ;;
      --no-frontend)     no_frontend=true; shift ;;
      --start-airbyte|--start-argo)
        echo "ERROR: $1 is not supported by the compose stack." >&2
        echo "       Both need k8s. Install orbstack/k3d/kind and use the" >&2
        echo "       existing ./dev-up.sh path. See CONTRIBUTING.md." >&2
        return 2 ;;
      -h|--help)         cmd_up_help; return 0 ;;
      *) echo "ERROR: unknown arg: $1" >&2; cmd_up_help; return 2 ;;
    esac
  done

  # First-run wizard: only when the user is using the default env file
  # and it doesn't exist yet. A custom --env-file path is left alone.
  if [[ "$env_file" == ".env.compose" && ! -f "$env_file" ]]; then
    bootstrap_env_wizard || return $?
  fi

  env_file="$(resolve_env_file "$env_file")"
  set -a; source "$env_file"; set +a

  [[ -n "$frontend_mode_override" ]] && FRONTEND_MODE="$frontend_mode_override"
  FRONTEND_MODE="${FRONTEND_MODE:-dev}"

  # ── Resolve which services go to ghcr ────────────────────────────
  local all_backend="api-gateway analytics-api identity"
  local ghcr_list=""
  local build_list=""

  [[ -n "${API_GATEWAY_IMAGE:-}"   ]] && ghcr_list=$(add "$ghcr_list" api-gateway)
  [[ -n "${ANALYTICS_API_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" analytics-api)
  [[ -n "${IDENTITY_IMAGE:-}"      ]] && ghcr_list=$(add "$ghcr_list" identity)

  if [[ -n "$from_ghcr_csv" ]]; then
    local OLD_IFS=$IFS; IFS=','
    local s
    for s in $from_ghcr_csv; do ghcr_list=$(add "$ghcr_list" "$(trim "$s")"); done
    IFS=$OLD_IFS
  fi
  if [[ -n "$build_only_csv" ]]; then
    local OLD_IFS=$IFS; IFS=','
    local s
    for s in $build_only_csv; do build_list=$(add "$build_list" "$(trim "$s")"); done
    IFS=$OLD_IFS
    for s in $all_backend; do
      contains "$build_list" "$s" || ghcr_list=$(add "$ghcr_list" "$s")
    done
  fi

  contains "$ghcr_list" api-gateway   && [[ -z "${API_GATEWAY_IMAGE:-}"   ]] && export API_GATEWAY_IMAGE="ghcr.io/constructorfabric/insight-api-gateway:${API_GATEWAY_GHCR_TAG:-latest}"
  contains "$ghcr_list" analytics-api && [[ -z "${ANALYTICS_API_IMAGE:-}" ]] && export ANALYTICS_API_IMAGE="ghcr.io/constructorfabric/insight-analytics-api:${ANALYTICS_API_GHCR_TAG:-latest}"
  contains "$ghcr_list" identity      && [[ -z "${IDENTITY_IMAGE:-}"      ]] && export IDENTITY_IMAGE="ghcr.io/constructorfabric/insight-identity:${IDENTITY_GHCR_TAG:-latest}"
  true

  # ── Generate per-run override ────────────────────────────────────
  local override="compose/override.generated.yml"
  mkdir -p compose
  {
    echo "# Auto-generated by dev-compose.sh — DO NOT EDIT BY HAND."
    echo "# Per-run override that flips selected services to ghcr mode."
    if [[ -z "$ghcr_list" ]]; then
      echo "services: {}"
    else
      echo "services:"
      local svc
      for svc in $all_backend; do
        if contains "$ghcr_list" "$svc"; then
          # Ghcr images are amd64-only for now (arm64 builds are
          # tracked separately). Pin the platform so Apple-silicon
          # hosts pull the amd64 manifest and run it under Rosetta
          # instead of erroring with "no matching manifest for
          # linux/arm64/v8".
          cat <<YML
  ${svc}:
    build: !reset null
    volumes: !override []
    entrypoint: !reset null
    command: !reset null
    platform: linux/amd64
YML
        fi
      done
    fi
  } > "$override"

  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml -f "$override")
  local profiles=()
  # Pull local DB services into scope unless the user pointed at an
  # external host. Backends use required:false on those depends_on
  # entries so an inactive profile is simply skipped.
  [[ "${MARIADB_EXTERNAL:-false}"    != "true" ]] && profiles+=(--profile local-mariadb)
  [[ "${CLICKHOUSE_EXTERNAL:-false}" != "true" ]] && profiles+=(--profile local-clickhouse)
  if [[ "$no_frontend" != "true" ]]; then
    case "$FRONTEND_MODE" in
      dev|built|ghcr) profiles+=(--profile "front-$FRONTEND_MODE") ;;
      *) echo "ERROR: FRONTEND_MODE must be dev|built|ghcr (got: $FRONTEND_MODE)" >&2; return 1 ;;
    esac
  fi

  # ── Build phase ──────────────────────────────────────────────────
  if [[ "$skip_build" != "true" ]]; then
    echo "=== Building artefacts (skip with --skip-build) ==="
    local rust_bins=""
    contains "$ghcr_list" api-gateway   || rust_bins="$rust_bins insight-api-gateway"
    contains "$ghcr_list" analytics-api || rust_bins="$rust_bins analytics-api"
    rust_bins=$(trim "$rust_bins")
    if [[ -n "$rust_bins" ]]; then
      echo "--- Rust:$rust_bins"
      local bin_flags=""
      local b
      for b in $rust_bins; do bin_flags="$bin_flags --bin $b"; done
      "${compose_cmd[@]}" --profile build run --rm \
        build-rust bash -c "
          set -eux
          apt-get update && apt-get install -y --no-install-recommends \
            protobuf-compiler libprotobuf-dev pkg-config libssl-dev > /dev/null
          cargo build --release$bin_flags
          mkdir -p /out/api-gateway /out/analytics-api
          [ -f /target/release/insight-api-gateway ] && install -m 0755 /target/release/insight-api-gateway /out/api-gateway/insight-api-gateway || true
          [ -f /target/release/analytics-api ]       && install -m 0755 /target/release/analytics-api       /out/analytics-api/analytics-api || true
        "
    fi
    if ! contains "$ghcr_list" identity; then
      echo "--- .NET: identity"
      "${compose_cmd[@]}" --profile build run --rm build-dotnet
    fi
    if [[ "$no_frontend" != "true" && "$FRONTEND_MODE" == "built" ]]; then
      echo "--- Frontend: pnpm build"
      "${compose_cmd[@]}" --profile build run --rm build-frontend
    fi
  fi

  local svc
  for svc in $all_backend; do
    contains "$ghcr_list" "$svc" && mkdir -p "compose/build/$svc"
  done

  echo "=== docker compose up ==="
  "${compose_cmd[@]}" ${profiles[@]+"${profiles[@]}"} up -d --remove-orphans

  echo
  "${compose_cmd[@]}" ps
  echo

  # ── First-run auto-seed ─────────────────────────────────────────────
  # Run seed once on the first up after the wizard. The SEEDED_LOCAL_*
  # markers in .env.compose are flipped to true on success so subsequent
  # `up` calls skip this block. For external DBs, the wizard pre-marks
  # them seeded unless the user explicitly opted in.
  local need_maria=false need_ch=false
  [[ "${SEEDED_LOCAL_MARIA:-}" != "true" ]] && need_maria=true
  [[ "${SEEDED_LOCAL_CH:-}"    != "true" ]] && need_ch=true
  if [[ "$need_maria" == "true" || "$need_ch" == "true" ]]; then
    local seed_target=""
    if   [[ "$need_maria" == "true" && "$need_ch" == "true" ]]; then seed_target=all
    elif [[ "$need_maria" == "true" ]]; then                          seed_target=identity
    else                                                              seed_target=silver
    fi
    echo "=== First-run seed ($seed_target) ==="
    if cmd_seed --env-file "$env_file" "$seed_target"; then
      [[ "$need_maria" == "true" ]] && update_env_var "$env_file" SEEDED_LOCAL_MARIA true
      [[ "$need_ch"    == "true" ]] && update_env_var "$env_file" SEEDED_LOCAL_CH    true
    else
      echo "WARN: seed failed; SEEDED_LOCAL_* not updated." >&2
      echo "      Re-run: ./dev-compose.sh seed $seed_target" >&2
    fi
    echo
  fi

  echo "Stop: ./dev-compose.sh down"
  echo "Rebuild one: ./dev-compose.sh build <service>"
  echo "Re-seed:     ./dev-compose.sh seed"
  echo "Wipe state:  ./dev-compose.sh prune"
}

# ──────────────────────────────────────────────────────────────────────
# down
# ──────────────────────────────────────────────────────────────────────

cmd_down_help() {
  cat <<'EOF'
usage: dev-compose.sh down [options]

Stop and remove every container. Data volumes (mariadb-data,
clickhouse-data, redis-data, redpanda-data, rust-target) are PRESERVED
unless --volumes is passed.

Options:
  --volumes  / -v  Also remove named volumes and wipe compose/build/.
  --env-file=PATH  Alternate dotenv file.
EOF
}

cmd_down() {
  local env_file=".env.compose"
  local wipe=false
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*) env_file="${1#*=}"; shift ;;
      --env-file)   env_file="$2"; shift 2 ;;
      --volumes|-v) wipe=true; shift ;;
      -h|--help)    cmd_down_help; return 0 ;;
      *) echo "ERROR: unknown arg: $1" >&2; cmd_down_help; return 2 ;;
    esac
  done
  env_file="$(resolve_env_file "$env_file")"

  local override="compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  "${compose_cmd[@]}" \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile build --profile seed \
    down $([[ "$wipe" == "true" ]] && echo "--volumes --remove-orphans")

  if [[ "$wipe" == "true" ]]; then
    echo "Wiping host-side build artefacts (compose/build/)..."
    rm -rf compose/build/
  fi
  echo "Done."
}

# ──────────────────────────────────────────────────────────────────────
# build
# ──────────────────────────────────────────────────────────────────────

cmd_build_help() {
  cat <<'EOF'
usage: dev-compose.sh build <target>

Rebuild one host-side artefact and let the already-running container
pick it up via ENABLE_AUTO_RELOAD.

Targets:
  api-gateway        Rust gateway binary only.
  analytics-api      Rust analytics binary only.
  identity           .NET 9 publish output.
  frontend           pnpm build → dist/.
  rust               Both Rust services.
  all                Everything (Rust + .NET + frontend).
EOF
}

cmd_build() {
  local env_file=".env.compose"
  if [[ "${1:-}" == "--env-file" ]]; then env_file="$2"; shift 2; fi
  if [[ "${1:-}" == --env-file=* ]]; then env_file="${1#*=}"; shift; fi

  local target="${1:-}"
  [[ -z "$target" || "$target" == "-h" || "$target" == "--help" ]] && { cmd_build_help; return 0; }

  env_file="$(resolve_env_file "$env_file")"
  set -a; source "$env_file"; set +a

  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml --profile build)
  build_rust_bins() {
    local bin_flags=""
    local b
    for b in "$@"; do bin_flags="$bin_flags --bin $b"; done
    "${compose_cmd[@]}" run --rm build-rust bash -c "
      set -eux
      apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler libprotobuf-dev pkg-config libssl-dev > /dev/null
      cargo build --release$bin_flags
      mkdir -p /out/api-gateway /out/analytics-api
      [ -f /target/release/insight-api-gateway ] && install -m 0755 /target/release/insight-api-gateway /out/api-gateway/insight-api-gateway || true
      [ -f /target/release/analytics-api ]       && install -m 0755 /target/release/analytics-api       /out/analytics-api/analytics-api || true
    "
  }

  case "$target" in
    api-gateway)   build_rust_bins insight-api-gateway ;;
    analytics-api) build_rust_bins analytics-api ;;
    rust)          build_rust_bins insight-api-gateway analytics-api ;;
    identity)      "${compose_cmd[@]}" run --rm build-dotnet ;;
    frontend)      "${compose_cmd[@]}" run --rm build-frontend ;;
    all)
      build_rust_bins insight-api-gateway analytics-api
      "${compose_cmd[@]}" run --rm build-dotnet
      "${compose_cmd[@]}" run --rm build-frontend
      ;;
    *) echo "ERROR: unknown target: $target" >&2; cmd_build_help; return 2 ;;
  esac
  echo "Done. If a runtime container has ENABLE_AUTO_RELOAD=true it will restart automatically."
}

# ──────────────────────────────────────────────────────────────────────
# seed
# ──────────────────────────────────────────────────────────────────────

cmd_seed_help() {
  cat <<'EOF'
usage: dev-compose.sh seed [identity|silver|all]

Populate the demo dataset. Stack must be up first.

  identity   25 persons + org_chart + account_person_map in MariaDB.
  silver     CREATE silver tables, apply gold-view migrations, generate
             ~24k rows of 60-day per-team activity in ClickHouse.
  all        Both (default if no arg).

After `silver` or `all` runs, analytics-api is restarted so its
metric-catalog schema validator re-checks the freshly-populated tables.
Without that bounce, every metric stays cached at the boot-time
`schema_status='error'`, the FE flags every bullet row schema_error=true,
and section badges read "no peer data" everywhere.
Tracking upstream as constructorfabric/insight#1307.

See compose/seed/README.md for the ruff/mypy/venv setup.
EOF
}

cmd_seed() {
  local env_file=".env.compose"
  if [[ "${1:-}" == "--env-file" ]]; then env_file="$2"; shift 2; fi
  if [[ "${1:-}" == --env-file=* ]]; then env_file="${1#*=}"; shift; fi
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then cmd_seed_help; return 0; fi

  env_file="$(resolve_env_file "$env_file")"
  local override="compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  local args=("$@")
  [[ ${#args[@]} -eq 0 ]] && args=("all")

  # Run the seed step itself. NOT `exec` — we still want to bounce
  # analytics-api after silver/all completes (see cf/insight#1307).
  "${compose_cmd[@]}" --profile seed run --rm seed-sample "${args[@]}"
  local seed_status=$?
  if [[ $seed_status -ne 0 ]]; then
    return $seed_status
  fi

  # Restart analytics-api when ClickHouse data was touched. Its schema
  # validator caches schema_status at startup and never re-checks; without
  # this nudge the catalog keeps serving the pre-seed 'table_not_found'
  # verdict and the FE shows "no peer data" everywhere.
  case "${args[0]}" in
    silver|all)
      echo
      echo "=== restarting analytics-api so it re-validates schema (cf/insight#1307) ==="
      "${compose_cmd[@]}" restart analytics-api >/dev/null
      ;;
  esac
}

# ──────────────────────────────────────────────────────────────────────
# prune
# ──────────────────────────────────────────────────────────────────────

cmd_prune_help() {
  cat <<'EOF'
usage: dev-compose.sh prune

DESTRUCTIVE — wipes local stack state. Interactive: you must approve
each step. There is no `--yes` switch on purpose.

The main pass removes:
  • all stack containers (insight-*)
  • named volumes: mariadb-data, clickhouse-data, clickhouse-logs,
    redis-data, redpanda-data, rust-target, frontend-node-modules
  • host-side build artefacts under compose/build/
  • generated compose/override.generated.yml
  • .env.compose

You will then be asked separately whether to also remove pulled
ghcr.io/constructorfabric/insight-* images (slow to re-pull; kept by
default).
EOF
}

cmd_prune() {
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then cmd_prune_help; return 0; fi

  cat <<EOF
This will permanently remove the local Insight stack state:
  • containers (insight-*)
  • named volumes (mariadb-data, clickhouse-data, redis-data,
    redpanda-data, rust-target, frontend-node-modules, ...)
  • compose/build/ artefacts
  • compose/override.generated.yml
  • .env.compose

EOF
  if ! ask_yes_no "Proceed?" "n"; then
    echo "Aborted." >&2
    return 1
  fi

  # We don't know which env file users picked; fall back to the example
  # if .env.compose is gone (e.g. after a partial prune).
  local env_file
  if [[ -f .env.compose ]]; then
    env_file=".env.compose"
  elif [[ -f .env.compose.example ]]; then
    env_file=".env.compose.example"
  else
    echo "ERROR: neither .env.compose nor .env.compose.example present." >&2
    return 1
  fi

  local override="compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  echo "=== docker compose down --volumes --remove-orphans ==="
  "${compose_cmd[@]}" \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile build --profile seed \
    --profile local-mariadb --profile local-clickhouse \
    down --volumes --remove-orphans || true

  if [[ -d compose/build ]]; then
    echo "Removing compose/build/..."
    rm -rf compose/build/
  fi
  if [[ -f "$override" ]]; then
    echo "Removing $override..."
    rm -f "$override"
  fi
  if [[ -f .env.compose ]]; then
    echo "Removing .env.compose..."
    rm -f .env.compose
  fi

  echo
  echo "Stack state wiped."
  echo

  # Image removal is a separate question — re-pulling is slow.
  if ask_yes_no "Also remove pulled ghcr.io/constructorfabric/insight-* images?" "n"; then
    local imgs
    imgs=$(docker images --format '{{.Repository}}:{{.Tag}}' 2>/dev/null \
           | grep -E '^ghcr\.io/constructorfabric/insight-' || true)
    if [[ -z "$imgs" ]]; then
      echo "  No matching images present."
    else
      echo "  Removing:"
      printf '    %s\n' $imgs
      # shellcheck disable=SC2086
      docker rmi $imgs || true
    fi
  fi

  echo
  echo "Done. Next ./dev-compose.sh up will re-run the first-run wizard."
}

# ──────────────────────────────────────────────────────────────────────
# Dispatcher
# ──────────────────────────────────────────────────────────────────────

usage() {
  cat <<'EOF'
usage: dev-compose.sh <subcommand> [args]

Subcommands:
  up      Build artefacts + start the stack. On first run it walks
          you through generating .env.compose.
  down    Stop everything. --volumes to wipe data.
  build   Rebuild one host-side artefact.
  seed    Populate the demo dataset (identity / silver / all).
  prune   Destructive wipe of containers, volumes, build/, override,
          and .env.compose. Asks for confirmation.
  help    Print this message.

Each subcommand has its own --help.
EOF
}

main() {
  local sub="${1:-help}"
  [[ $# -gt 0 ]] && shift
  case "$sub" in
    up)    cmd_up    "$@" ;;
    down)  cmd_down  "$@" ;;
    build) cmd_build "$@" ;;
    seed)  cmd_seed  "$@" ;;
    prune) cmd_prune "$@" ;;
    help|-h|--help) usage ;;
    *) echo "ERROR: unknown subcommand: $sub" >&2; usage; return 2 ;;
  esac
}

main "$@"

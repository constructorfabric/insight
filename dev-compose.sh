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
# Helpers that survived the wizard extraction
#
# The first-run wizard moved to deploy/compose/insight-init.sh (shared with the
# k8s-local bring-up). These two helpers stay because non-wizard
# subcommands here (prune, cmd_up's seed-gate flip) still use them.
# ──────────────────────────────────────────────────────────────────────

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
  escaped=$(printf '%s' "$value" | sed -e 's/[\\&|]/\\&/g')
  if grep -qE "^[[:space:]]*${key}=" "$file" 2>/dev/null; then
    tmp=$(mktemp)
    sed -E "s|^[[:space:]]*${key}=.*|${key}=${escaped}|" "$file" > "$tmp"
    mv "$tmp" "$file"
  else
    printf '%s=%s\n' "$key" "$value" >> "$file"
  fi
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
                            analytics, identity.
  --build-only=svc1,svc2    Build only these; everything else from ghcr.
  --frontend-mode=MODE      Override FRONTEND_MODE for this run.
                            (dev | built | ghcr)
  --no-frontend             Don't start any frontend variant.
  --skip-build              Don't rebuild artefacts — reuse what's
                            already in deploy/compose/build/.
  --env-file=PATH           Alternate dotenv file. Default: .env.compose.

Out-of-scope:
  --start-airbyte / --start-argo
      Both need k8s and are not shipped by this compose stack. For a
      k8s-local bring-up that includes Airbyte and Argo Workflows, run
      `make deploy ENV=local` from deploy/gitops/.
EOF
}

# Generate the dev-only ES256 signing key the authenticator mounts at
# signing_keys_path. Never committed (gitignored) and never baked into an
# image; regenerated on demand. Prod mounts a real key via a K8s Secret.
ensure_authenticator_dev_key() {
  local dir="deploy/compose/authenticator-dev-keys"
  local key="$dir/current.pem"
  # Reuse an existing key only if it is a usable named-curve P-256 key. A key
  # generated by an older dev-compose.sh on LibreSSL carries explicit EC
  # parameters the authenticator's p256 loader rejects — regenerate those.
  if [[ -f "$key" ]]; then
    openssl asn1parse -in "$key" 2>/dev/null | grep -q prime256v1 && return 0
    echo "=== Regenerating authenticator dev key ($key): not named-curve P-256 ===" >&2
    rm -f "$key"
  fi
  mkdir -p "$dir"
  echo "=== Generating dev ES256 signing key for the authenticator ($key) ==="
  # ec_param_enc:named_curve is REQUIRED: LibreSSL (macOS default openssl)
  # otherwise emits explicit EC parameters, which the authenticator's p256
  # PKCS#8 loader rejects ("expected OBJECT IDENTIFIER, got SEQUENCE").
  if ! openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve -out "$key" 2>/dev/null; then
    echo "WARN: openssl unavailable — the authenticator will fail to start without $key" >&2
    return 1
  fi
  chmod 600 "$key"
  ensure_service_token_dev_key "$dir"
}

# Generate the dev-only service-token keypair (registry entry `testclient`).
# The authenticator reads only the public half (mounted, referenced by
# public_key_paths); the private half is for a calling service / manual testing.
# Never committed (same gitignored dir as the signing key).
ensure_service_token_dev_key() {
  local dir="$1"
  local key="$dir/testclient.key.pem"
  local pub="$dir/testclient.pub.pem"
  [[ -f "$pub" ]] && return 0
  echo "=== Generating dev service-token keypair for the authenticator ($pub) ==="
  # ec_param_enc:named_curve for the same LibreSSL reason as the signing key above.
  if ! openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve -out "$key" 2>/dev/null; then
    echo "WARN: openssl unavailable — service tokens will not work without $pub" >&2
    return 1
  fi
  openssl pkey -in "$key" -pubout -out "$pub" 2>/dev/null
  chmod 600 "$key"
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
        echo "       Both need k8s. Bring up a kind/k3d/OrbStack cluster, then:" >&2
        echo "         cd deploy/gitops && make deploy ENV=local" >&2
        echo "       The first-run wizard prompts for which L2 services to install." >&2
        return 2 ;;
      -h|--help)         cmd_up_help; return 0 ;;
      *) echo "ERROR: unknown arg: $1" >&2; cmd_up_help; return 2 ;;
    esac
  done

  # First-run wizard: only when the user is using the default env file
  # and it doesn't exist yet. A custom --env-file path is left alone.
  # The wizard itself lives in deploy/compose/insight-init.sh, shared with the
  # k8s-local bring-up.
  if [[ "$env_file" == ".env.compose" && ! -f "$env_file" ]]; then
    bash "$ROOT_DIR/deploy/compose/insight-init.sh" --target=compose || return $?
  fi

  env_file="$(resolve_env_file "$env_file")"
  set -a; source "$env_file"; set +a

  [[ -n "$frontend_mode_override" ]] && FRONTEND_MODE="$frontend_mode_override"
  FRONTEND_MODE="${FRONTEND_MODE:-dev}"

  # ── Resolve which services go to ghcr ────────────────────────────
  local all_backend="api-gateway analytics identity"
  local ghcr_list=""
  local build_list=""

  [[ -n "${API_GATEWAY_IMAGE:-}"   ]] && ghcr_list=$(add "$ghcr_list" api-gateway)
  [[ -n "${ANALYTICS_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" analytics)
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
  contains "$ghcr_list" analytics && [[ -z "${ANALYTICS_IMAGE:-}" ]] && export ANALYTICS_IMAGE="ghcr.io/constructorfabric/insight-analytics:${ANALYTICS_GHCR_TAG:-latest}"
  contains "$ghcr_list" identity      && [[ -z "${IDENTITY_IMAGE:-}"      ]] && export IDENTITY_IMAGE="ghcr.io/constructorfabric/insight-identity:${IDENTITY_GHCR_TAG:-latest}"
  true

  # ── Generate per-run override ────────────────────────────────────
  local override="deploy/compose/override.generated.yml"
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

  # Ensure the authenticator's dev signing key exists before bring-up.
  ensure_authenticator_dev_key

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
    # authenticator is always built from source (no ghcr flip for it) and its
    # binary is bind-mounted as a file — omit it and compose auto-creates the
    # mount source as an empty directory, failing container init.
    local rust_bins="authenticator"
    contains "$ghcr_list" api-gateway || rust_bins="$rust_bins insight-api-gateway"
    contains "$ghcr_list" analytics   || rust_bins="$rust_bins analytics"
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
          mkdir -p /out/api-gateway /out/analytics /out/authenticator
          [ -f /target/release/insight-api-gateway ] && install -m 0755 /target/release/insight-api-gateway /out/api-gateway/insight-api-gateway || true
          [ -f /target/release/analytics ]           && install -m 0755 /target/release/analytics           /out/analytics/analytics || true
          [ -f /target/release/authenticator ]       && install -m 0755 /target/release/authenticator       /out/authenticator/authenticator || true
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
    contains "$ghcr_list" "$svc" && mkdir -p "deploy/compose/build/$svc"
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

  local frontend_up=true
  [[ "$no_frontend" == "true" ]] && frontend_up=false
  report_service_urls "$frontend_up"
  echo

  echo "Service URLs: ./dev-compose.sh urls"
  echo "Stop:        ./dev-compose.sh down"
  echo "Rebuild one: ./dev-compose.sh build <service>"
  echo "Re-seed:     ./dev-compose.sh seed"
  echo "Wipe state:  ./dev-compose.sh prune"
}

# ──────────────────────────────────────────────────────────────────────
# Service access report
# ──────────────────────────────────────────────────────────────────────

# Print how to reach every exposed service on the host, honouring the
# configurable ports (and defaults) from .env.compose / docker-compose.yml.
# Callers must have sourced the env file first. Local-only DBs are shown
# unless pointed at an external host; the frontend line is gated by the
# caller (arg 1 = "true" when a front-* profile is active).
report_service_urls() {
  local frontend_up="${1:-true}"
  local h="localhost"
  echo "=== Service URLs (exposed host ports) ==="
  if [[ "$frontend_up" == "true" ]]; then
    printf '  %-18s %s\n' "Frontend UI"   "http://$h:${FRONTEND_PORT:-3000}"
  fi
  printf '  %-18s %s\n' "API Gateway"     "http://$h:${API_GATEWAY_PORT:-8080}"
  printf '  %-18s %s\n' "Analytics API"   "http://$h:${ANALYTICS_PORT:-8081}"
  printf '  %-18s %s\n' "Identity API"    "http://$h:${IDENTITY_PORT:-8082}"
  printf '  %-18s %s\n' "Authenticator"   "http://$h:${AUTHENTICATOR_PORT:-8083}"
  printf '  %-18s %s\n' "Fake IdP"        "http://$h:${FAKEIDP_PORT:-8084}"
  if [[ "${CLICKHOUSE_EXTERNAL:-false}" != "true" ]]; then
    printf '  %-18s %s\n' "ClickHouse HTTP" \
      "http://$h:${CLICKHOUSE_HTTP_PORT:-8123}  (native $h:${CLICKHOUSE_NATIVE_PORT:-9000}, user ${CLICKHOUSE_USER:-insight})"
  fi
  if [[ "${MARIADB_EXTERNAL:-false}" != "true" ]]; then
    printf '  %-18s %s\n' "MariaDB"        "$h:${MARIADB_PORT:-3306}  (user ${MARIADB_USER:-insight})"
  fi
  printf '  %-18s %s\n' "Redis"           "$h:${REDIS_PORT:-6379}"
  printf '  %-18s %s\n' "Redpanda Kafka"  \
    "$h:${REDPANDA_KAFKA_PORT:-19092}  (admin $h:${REDPANDA_ADMIN_PORT:-19644}, schema $h:${REDPANDA_SCHEMA_PORT:-18081})"
}

# ──────────────────────────────────────────────────────────────────────
# urls
# ──────────────────────────────────────────────────────────────────────

cmd_urls() {
  local env_file=".env.compose"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*) env_file="${1#*=}"; shift ;;
      --env-file)   env_file="$2"; shift 2 ;;
      -h|--help)    echo "usage: dev-compose.sh urls [--env-file FILE]"; return 0 ;;
      *) echo "ERROR: unknown arg: $1" >&2; return 2 ;;
    esac
  done
  env_file="$(resolve_env_file "$env_file")" || return $?
  set -a; source "$env_file"; set +a
  # FRONTEND_MODE is always dev|built|ghcr (cmd_up enforces it), so the
  # frontend is assumed up; report_service_urls defaults to showing it.
  report_service_urls
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
  --volumes  / -v  Also remove named volumes and wipe deploy/compose/build/.
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

  local override="deploy/compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  "${compose_cmd[@]}" \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile build --profile seed \
    down $([[ "$wipe" == "true" ]] && echo "--volumes --remove-orphans")

  if [[ "$wipe" == "true" ]]; then
    echo "Wiping host-side build artefacts (deploy/compose/build/)..."
    rm -rf deploy/compose/build/
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
  analytics          Rust analytics binary only.
  authenticator      Rust authenticator binary only.
  identity           .NET 9 publish output.
  frontend           pnpm build → dist/.
  rust               All three Rust services.
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
      mkdir -p /out/api-gateway /out/analytics /out/authenticator
      [ -f /target/release/insight-api-gateway ] && install -m 0755 /target/release/insight-api-gateway /out/api-gateway/insight-api-gateway || true
      [ -f /target/release/analytics ]           && install -m 0755 /target/release/analytics           /out/analytics/analytics || true
      [ -f /target/release/authenticator ]       && install -m 0755 /target/release/authenticator       /out/authenticator/authenticator || true
    "
  }

  case "$target" in
    api-gateway)   build_rust_bins insight-api-gateway ;;
    analytics)     build_rust_bins analytics ;;
    authenticator) build_rust_bins authenticator ;;
    rust)          build_rust_bins insight-api-gateway analytics authenticator ;;
    identity)    "${compose_cmd[@]}" run --rm build-dotnet ;;
    frontend)    "${compose_cmd[@]}" run --rm build-frontend ;;
    all)
      build_rust_bins insight-api-gateway analytics authenticator
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

After `silver` or `all` runs, analytics is restarted so its
metric-catalog schema validator re-checks the freshly-populated tables.
Without that bounce, every metric stays cached at the boot-time
`schema_status='error'`, the FE flags every bullet row schema_error=true,
and section badges read "no peer data" everywhere.
Tracking upstream as constructorfabric/insight#1307.

See deploy/seed/README.md for the ruff/mypy/venv setup.
EOF
}

cmd_seed() {
  local env_file=".env.compose"
  if [[ "${1:-}" == "--env-file" ]]; then env_file="$2"; shift 2; fi
  if [[ "${1:-}" == --env-file=* ]]; then env_file="${1#*=}"; shift; fi
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then cmd_seed_help; return 0; fi

  env_file="$(resolve_env_file "$env_file")"
  local override="deploy/compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  local args=("$@")
  [[ ${#args[@]} -eq 0 ]] && args=("all")

  # Run the seed step itself. NOT `exec` — we still want to bounce
  # analytics after silver/all completes (see cf/insight#1307).
  "${compose_cmd[@]}" --profile seed run --rm seed-sample "${args[@]}"
  local seed_status=$?
  if [[ $seed_status -ne 0 ]]; then
    return $seed_status
  fi

  # Restart analytics when ClickHouse data was touched. Its schema
  # validator caches schema_status at startup and never re-checks; without
  # this nudge the catalog keeps serving the pre-seed 'table_not_found'
  # verdict and the FE shows "no peer data" everywhere.
  case "${args[0]}" in
    silver|all)
      echo
      echo "=== restarting analytics so it re-validates schema (cf/insight#1307) ==="
      "${compose_cmd[@]}" restart analytics >/dev/null
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
  • host-side build artefacts under deploy/compose/build/
  • the generated authenticator dev signing key
    (deploy/compose/authenticator-dev-keys/)
  • generated deploy/compose/override.generated.yml
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
  • deploy/compose/build/ artefacts
  • deploy/compose/authenticator-dev-keys/ (dev signing key)
  • deploy/compose/override.generated.yml
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

  local override="deploy/compose/override.generated.yml"
  local compose_cmd=(docker compose --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  echo "=== docker compose down --volumes --remove-orphans ==="
  "${compose_cmd[@]}" \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile build --profile seed \
    --profile local-mariadb --profile local-clickhouse \
    down --volumes --remove-orphans || true

  if [[ -d deploy/compose/build ]]; then
    echo "Removing deploy/compose/build/..."
    rm -rf deploy/compose/build/
  fi
  if [[ -d deploy/compose/authenticator-dev-keys ]]; then
    echo "Removing deploy/compose/authenticator-dev-keys/ (dev signing key)..."
    rm -rf deploy/compose/authenticator-dev-keys/
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
  urls    Print how to reach each service (exposed host ports).
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
    urls)  cmd_urls  "$@" ;;
    prune) cmd_prune "$@" ;;
    help|-h|--help) usage ;;
    *) echo "ERROR: unknown subcommand: $sub" >&2; usage; return 2 ;;
  esac
}

main "$@"

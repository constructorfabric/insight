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
                            of building. Recognised:
                            analytics, identity.
  --watch=svc1,svc2         Run selected Rust services from source with
                            cargo-watch. Recognised: analytics.
  --build-only=svc1,svc2    Build only these; everything else from ghcr.
  --frontend-mode=MODE      Override FRONTEND_MODE for this run.
                            (dev | built | ghcr)
  --auth=MODE               Override AUTH_MODE (fakeidp|keycloak) from
                            .env.compose for this run only.
                            (fakeidp | keycloak, default: fakeidp)
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
# signing_keys_path (§9.6). Never committed (gitignored) and never baked into an
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

# Generate the dev-only self-signed TLS cert for the `authn-tls` front (SAN
# authn-tls). The analytics oidc-authn-plugin resolves the authenticator's JWKS
# via OIDC discovery over https ONLY; authn-tls terminates that TLS and analytics
# trusts ca.pem. Never committed (gitignored). Regenerated when missing/expired.
ensure_authn_tls_certs() {
  local dir="deploy/compose/authn-tls-certs"
  local cert="$dir/server.pem"
  [[ -f "$cert" ]] && openssl x509 -in "$cert" -noout -checkend 86400 2>/dev/null && return 0
  mkdir -p "$dir"
  echo "=== Generating dev TLS cert for the authn-tls discovery front ($cert) ==="
  # A config file (not -addext) keeps this working on LibreSSL (macOS default).
  local cnf="$dir/openssl.cnf"
  cat > "$cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no
[dn]
CN = authn-tls
[v3]
subjectAltName = DNS:authn-tls
EOF
  if ! openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve -out "$dir/server.key" 2>/dev/null; then
    echo "WARN: openssl unavailable — analytics cannot verify the gateway JWT without the authn-tls cert" >&2
    return 1
  fi
  openssl req -x509 -key "$dir/server.key" -out "$cert" -days 3650 -config "$cnf" 2>/dev/null
  # The self-signed leaf is its own trust root (analytics adds it as a CA).
  cp "$cert" "$dir/ca.pem"
  chmod 644 "$dir/server.key" "$cert" "$dir/ca.pem"
}

# The host's primary IPv4 (macOS: the default-route interface; Linux: the src of
# the default route). An IP LITERAL — browsers don't HTTPS-upgrade it (unlike a
# hostname), and it's reachable from both the host browser and the containers.
detect_host_ip() {
  if command -v ipconfig >/dev/null 2>&1; then           # macOS
    local ifc
    ifc="$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')"
    if [[ -n "$ifc" ]] && ipconfig getifaddr "$ifc" 2>/dev/null; then return 0; fi
    ipconfig getifaddr en0 2>/dev/null && return 0
    return 1
  fi
  ip route get 1.1.1.1 2>/dev/null \
    | awk '{for (i=1;i<=NF;i++) if ($i=="src") {print $(i+1); exit}}'
}

# Point fakeidp's issuer at the host IP so the BROWSER login flow works out of
# the box. The authenticator 302s the browser to `{issuer}/authorize`; a
# hostname (`fakeidp:8084`) gets HTTPS-upgraded by the browser and fails (fakeidp
# is http-only), and `localhost` means the container itself. The host IP
# satisfies both sides over plain http. fakeidp's advertised issuer and the
# authenticator's expected issuer MUST match, so set both. Skipped when the
# operator pinned an issuer (a real IdP) or when no host IP is detectable
# (offline) — then it stays `fakeidp:8084`, which still serves the curl/e2e path.
ensure_fakeidp_issuer() {
  [[ -n "${AUTHENTICATOR_OIDC_ISSUER:-}" ]] && return 0
  local ip
  ip="$(detect_host_ip || true)"
  if [[ -z "$ip" ]]; then
    echo "WARN: no host IP detected — fakeidp issuer stays http://fakeidp:8084." >&2
    echo "      curl/e2e still work; browser login needs the browser's HTTPS-upgrade off." >&2
    return 0
  fi
  export FAKEIDP_ISSUER="http://$ip:8084"
  export AUTHENTICATOR_OIDC_ISSUER="http://$ip:8084"
  echo "fakeidp issuer → http://$ip:8084 (host IP; browser-reachable, no HTTPS upgrade)"
}

write_watch_override() {
  local svc="$1"
  case "$svc" in
    analytics)
      cat <<'YML'
  analytics:
    image: insight-rust-watch:dev
    pull_policy: build
    build:
      context: deploy/compose
      dockerfile: rust-watch.Dockerfile
    entrypoint: !reset null
    working_dir: /workspace
    environment:
      ENABLE_AUTO_RELOAD: ""
      CARGO_TARGET_DIR: /target
      CARGO_INCREMENTAL: "1"
    volumes: !override
      - ./src/backend:/workspace:ro
      - rust-target:/target
      - rust-cargo-registry:/usr/local/cargo/registry
      - rust-cargo-git:/usr/local/cargo/git
      - ./deploy/compose/analytics-fullauth.yaml:/app/config/insight.yaml:ro
      - ./deploy/compose/authn-tls-certs:/certs:ro
    command:
      - cargo-watch
      - --poll
      - --exec
      - run --bin analytics -- -c /app/config/insight.yaml run
YML
      ;;
    *)
      echo "ERROR: no watch configuration registered for service '$svc'." >&2
      return 1
      ;;
  esac
}

cmd_up() {
  local env_file=".env.compose"
  local from_ghcr_csv=""
  local watch_csv=""
  local watch_option_set=false
  local build_only_csv=""
  local frontend_mode_override=""
  local auth_mode_override=""
  local skip_build=false
  local no_frontend=false

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*)      env_file="${1#*=}"; shift ;;
      --env-file)        env_file="$2"; shift 2 ;;
      --from-ghcr=*)     from_ghcr_csv="${1#*=}"; shift ;;
      --from-ghcr)       from_ghcr_csv="$2"; shift 2 ;;
      --watch=*)         watch_csv="${1#*=}"; watch_option_set=true; shift ;;
      --watch)
        [[ $# -ge 2 ]] || { echo "ERROR: --watch requires a value." >&2; return 2; }
        watch_csv="$2"; watch_option_set=true; shift 2 ;;
      --build-only=*)    build_only_csv="${1#*=}"; shift ;;
      --build-only)      build_only_csv="$2"; shift 2 ;;
      --frontend-mode=*) frontend_mode_override="${1#*=}"; shift ;;
      --frontend-mode)   frontend_mode_override="$2"; shift 2 ;;
      --auth=*)          auth_mode_override="${1#*=}"; shift ;;
      --auth)            auth_mode_override="$2"; shift 2 ;;
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
    local init_args=(--target=compose)
    [[ "$no_frontend" == "true" ]] && init_args+=(--no-frontend)
    bash "$ROOT_DIR/deploy/compose/insight-init.sh" "${init_args[@]}" || return $?
  fi

  env_file="$(resolve_env_file "$env_file")"
  set -a; source "$env_file"; set +a

  if [[ -n "${VITE_DEV_USER_EMAIL:-}" && -z "${DEV_USER_EMAIL:-}" ]]; then
    echo "ERROR: VITE_DEV_USER_EMAIL was renamed to DEV_USER_EMAIL." >&2
    echo "       Update $env_file before running the stack." >&2
    return 1
  fi
  : "${DEV_USER_EMAIL:?DEV_USER_EMAIL must be set (for example, dev@company.nonpresent)}"

  [[ -n "$frontend_mode_override" ]] && FRONTEND_MODE="$frontend_mode_override"
  FRONTEND_MODE="${FRONTEND_MODE:-dev}"

  [[ -n "$auth_mode_override" ]] && AUTH_MODE="$auth_mode_override"
  AUTH_MODE="${AUTH_MODE:-fakeidp}"  # RULE-DEFAULTS-OK: fakeidp is the documented default auth mode (bypass)
  case "$AUTH_MODE" in
    fakeidp|keycloak) ;;
    *) echo "ERROR: AUTH_MODE must be fakeidp|keycloak (got: $AUTH_MODE)" >&2; return 1 ;;
  esac

  # Browser OIDC: default the fakeidp issuer to the host IP (unless pinned).
  # keycloak mode sets its own host-IP issuer in the AUTH_MODE=keycloak block.
  [[ "$AUTH_MODE" == fakeidp ]] && ensure_fakeidp_issuer

  # NGINX_BFF: keycloak mode needs NO special frontend. The SPA is cookie/BFF
  # (same-origin): it calls /auth/login + /api through the gateway and never
  # does client-side OIDC, so any FRONTEND_MODE (incl. the default `dev` Vite)
  # works — the authenticator, not the frontend, drives the Keycloak login.

  # ── Resolve which services go to ghcr ────────────────────────────
  # The legacy Rust api-gateway is gone; the nginx `gateway` is the sole :8080
  # entry doing full auth via the authenticator (NGINX_BFF #1583 step 09).
  local all_backend="analytics identity"
  local watchable_services="analytics"
  local ghcr_list=""
  local watch_list=""
  local build_list=""

  [[ -n "${ANALYTICS_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" analytics)
  [[ -n "${IDENTITY_IMAGE:-}"      ]] && ghcr_list=$(add "$ghcr_list" identity)

  if [[ -n "$from_ghcr_csv" ]]; then
    local OLD_IFS=$IFS; IFS=','
    local s
    for s in $from_ghcr_csv; do ghcr_list=$(add "$ghcr_list" "$(trim "$s")"); done
    IFS=$OLD_IFS
  fi
  if [[ "$watch_option_set" == "true" ]]; then
    case "$watch_csv" in
      ""|,*|*,|*,,*) echo "ERROR: --watch requires a comma-separated service list without empty entries." >&2; return 2 ;;
    esac
    local OLD_IFS=$IFS; IFS=','
    local s
    for s in $watch_csv; do
      s="$(trim "$s")"
      [[ -n "$s" ]] || { echo "ERROR: --watch contains an empty service name." >&2; return 2; }
      contains "$watchable_services" "$s" || {
        echo "ERROR: service '$s' does not support --watch (supported: $watchable_services)." >&2
        return 2
      }
      watch_list=$(add "$watch_list" "$s")
    done
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

  local s
  for s in $watch_list; do
    if contains "$ghcr_list" "$s"; then
      echo "ERROR: service '$s' cannot use both --watch and --from-ghcr/image override." >&2
      return 2
    fi
  done

  contains "$ghcr_list" analytics && [[ -z "${ANALYTICS_IMAGE:-}" ]] && export ANALYTICS_IMAGE="ghcr.io/constructorfabric/insight-analytics:${ANALYTICS_GHCR_TAG:-latest}"
  contains "$ghcr_list" identity      && [[ -z "${IDENTITY_IMAGE:-}"      ]] && export IDENTITY_IMAGE="ghcr.io/constructorfabric/insight-identity:${IDENTITY_GHCR_TAG:-latest}"
  true

  # ── Generate per-run override ────────────────────────────────────
  local override="deploy/compose/override.generated.yml"
  mkdir -p compose
  local want_overrides=false
  [[ -n "$ghcr_list" || -n "$watch_list" ]] && want_overrides=true
  {
    echo "# Auto-generated by dev-compose.sh — DO NOT EDIT BY HAND."
    echo "# Per-run override for selected service execution modes."
    if [[ "$want_overrides" != true ]]; then
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
        elif contains "$watch_list" "$svc"; then
          write_watch_override "$svc"
        fi
      done
      # NGINX_BFF: no frontend dev-impersonation to disable — the cookie/BFF SPA
      # has no impersonation path, so keycloak mode needs no per-frontend override.
    fi
  } > "$override"

  # Ensure the authenticator's dev signing key + the authn-tls discovery cert
  # exist before bring-up (full-auth: analytics verifies the gateway JWT).
  ensure_authenticator_dev_key
  ensure_authn_tls_certs

  # Keycloak mode: generate the realm import file and repoint the
  # authenticator's BFF at Keycloak. This must run before `up -d` — the
  # keycloak service read-only-mounts the generated file, and if it's
  # missing at container-create time Docker creates an empty directory
  # at the mount path instead, so --import-realm silently imports nothing.
  if [[ "$AUTH_MODE" == keycloak ]]; then
    # Roster anchor for the realm's dev-lead persona. The realm roster and the
    # seed step both need it.
    local dev_lead_email="${DEV_USER_EMAIL:?DEV_USER_EMAIL must be set (roster anchor for the Keycloak realm; e.g. dev@company.nonpresent — see .env.compose)}"

    # The authenticator (server-side) AND the browser must reach Keycloak at the
    # SAME issuer, or the id_token `iss` won't validate. Use the host IP (an IP
    # literal the browser won't HTTPS-upgrade, reachable from the container via
    # the published :8085) — the same trick as ensure_fakeidp_issuer. A
    # `localhost` issuer is unreachable from inside the authenticator; a
    # `keycloak:8085` issuer wouldn't match the browser-facing `iss`.
    local kc_ip; kc_ip="$(detect_host_ip || true)"
    if [[ -z "$kc_ip" ]]; then
      echo "WARN: no host IP detected — Keycloak issuer stays localhost (browser-only; the authenticator can't reach it)." >&2
      kc_ip="localhost"
    fi
    local kc_base="http://${kc_ip}:8085/kc"

    echo "=== Generating Keycloak realm import (deploy/compose/keycloak/realm-insight.generated.json) ==="
    python3 deploy/compose/keycloak/gen-realm.py \
      --dev-email "$dev_lead_email" \
      --out deploy/compose/keycloak/realm-insight.generated.json

    # NGINX_BFF: the AUTHENTICATOR (not the frontend) logs in against Keycloak,
    # server-side, as the pre-seeded `insight-authenticator` confidential client.
    # - KEYCLOAK_HOSTNAME  -> the keycloak service's advertised (browser-facing) issuer
    # - AUTHENTICATOR_OIDC_ISSUER -> what the authenticator discovers + validates `iss` against
    # redirect_uri keeps its default (the SPA origin http://localhost:3000/auth/callback,
    # which the realm registers for this client).
    export KEYCLOAK_HOSTNAME="$kc_base"
    export AUTHENTICATOR_OIDC_ISSUER="${kc_base}/realms/insight"
    export OIDC_CLIENT_ID="insight-authenticator"
    export OIDC_CLIENT_SECRET="insight-authenticator-dev-secret"
    echo "keycloak issuer → ${kc_base}/realms/insight (host IP; browser + authenticator reachable)"

    # AUTH_DISABLED is a separate, blunter bypass; if it's on, real login is
    # still skipped regardless.
    [[ "${AUTH_DISABLED:-false}" == "true" ]] && {  # RULE-DEFAULTS-OK: purely a cosmetic warn-or-not check, not a config value
      echo "WARN: AUTH_DISABLED=true forces an auth bypass — unset it to" >&2
      echo "      exercise the real Keycloak login flow." >&2
    }
  fi

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
  profiles+=(--profile "auth-$AUTH_MODE")

  # ── Build phase ──────────────────────────────────────────────────
  if [[ "$skip_build" != "true" ]]; then
    echo "=== Building artefacts (skip with --skip-build) ==="
    if [[ "$AUTH_MODE" == fakeidp ]]; then
      echo "--- Image: fakeidp"
      "${compose_cmd[@]}" --profile auth-fakeidp build fakeidp
    fi
    # authenticator is always built from source (no ghcr flip for it) and its
    # binary is bind-mounted as a file — omit it and compose auto-creates the
    # mount source as an empty directory, failing container init.
    local rust_bins="authenticator"
    contains "$ghcr_list" analytics || contains "$watch_list" analytics || rust_bins="$rust_bins analytics"
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
          mkdir -p /out/analytics /out/authenticator
          if [ -f /target/release/analytics ]; then
            [ ! -d /out/analytics/analytics ] || rm -rf /out/analytics/analytics
            install -m 0755 /target/release/analytics /out/analytics/analytics
          fi
          if [ -f /target/release/authenticator ]; then
            [ ! -d /out/authenticator/authenticator ] || rm -rf /out/authenticator/authenticator
            install -m 0755 /target/release/authenticator /out/authenticator/authenticator
          fi
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

  # Stop the OTHER auth mode's IdP if it lingers from a prior in-place `up`.
  # Compose profiles decide what to START, not what to stop, so switching auth
  # modes without a `down` in between would otherwise leave both IdPs running
  # (e.g. fakeidp still up after switching to keycloak). Pass both auth
  # profiles so the target service is in scope for `stop`.
  local other_idp
  [[ "$AUTH_MODE" == keycloak ]] && other_idp=fakeidp || other_idp=keycloak
  "${compose_cmd[@]}" --profile auth-fakeidp --profile auth-keycloak stop "$other_idp" >/dev/null 2>&1 || true

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
  report_service_urls "$frontend_up" "$AUTH_MODE"
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
  local auth_mode="${2:-fakeidp}"
  local h="localhost"
  echo "=== Service URLs (exposed host ports) ==="
  if [[ "$frontend_up" == "true" ]]; then
    printf '  %-18s %s\n' "Frontend UI"   "http://$h:${FRONTEND_PORT:-3000}"
  fi
  printf '  %-18s %s\n' "Gateway"         "http://$h:${GATEWAY_PORT:-8080}"
  printf '  %-18s %s\n' "Analytics API"   "http://$h:${ANALYTICS_PORT:-8081}"
  printf '  %-18s %s\n' "Identity API"    "http://$h:${IDENTITY_PORT:-8082}"
  printf '  %-18s %s\n' "Authenticator"   "http://$h:${AUTHENTICATOR_PORT:-8083}"
  if [[ "$auth_mode" == keycloak ]]; then
    printf '  %-18s %s\n' "Keycloak" \
      "http://$h:${KEYCLOAK_PORT:-8085}/kc/admin/  (admin console: admin/admin)"  # RULE-DEFAULTS-OK: display-only port default, mirrors the pre-existing per-service *_PORT lines above
  else
    printf '  %-18s %s\n' "Fake IdP"        "http://$h:${FAKEIDP_PORT:-8084}"
  fi
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

  echo
  echo "=== Sign in ==="
  if [[ "$frontend_up" != "true" ]]; then
    if [[ "$auth_mode" == keycloak ]]; then
      echo "  Frontend is not running (--no-frontend); browser sign-in is unavailable."
    else
      echo "  fakeidp is configured to log in as ${DEV_USER_EMAIL:-dev@company.nonpresent}; frontend is not running (--no-frontend)."
    fi
    return
  fi
  if [[ "$auth_mode" == keycloak ]]; then
    echo "  Open http://$h:${FRONTEND_PORT:-3000}, click Sign in, then at the Keycloak form enter"
    echo "  your dev persona (or any seeded user) + password insight-dev:"
    echo "    ${DEV_USER_EMAIL:-dev@company.nonpresent}   /   insight-dev"
  else
    echo "  fakeidp auto-logs-in as ${DEV_USER_EMAIL:-dev@company.nonpresent} (no form) — just open http://$h:${FRONTEND_PORT:-3000}."
  fi
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
  AUTH_MODE="${AUTH_MODE:-fakeidp}"  # RULE-DEFAULTS-OK: fakeidp is the documented default auth mode (bypass)
  # FRONTEND_MODE is always dev|built|ghcr (cmd_up enforces it), so the
  # frontend is assumed up; report_service_urls defaults to showing it.
  report_service_urls true "$AUTH_MODE"
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
    --profile auth-fakeidp --profile auth-keycloak \
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
      mkdir -p /out/analytics /out/authenticator
      if [ -f /target/release/analytics ]; then
        [ ! -d /out/analytics/analytics ] || rm -rf /out/analytics/analytics
        install -m 0755 /target/release/analytics /out/analytics/analytics
      fi
      if [ -f /target/release/authenticator ]; then
        [ ! -d /out/authenticator/authenticator ] || rm -rf /out/authenticator/authenticator
        install -m 0755 /target/release/authenticator /out/authenticator/authenticator
      fi
    "
  }

  # Accept MULTIPLE targets, e.g. `build authenticator identity`. Rust bins are
  # batched into one build; dotnet/frontend run once if requested.
  local rust_bins="" want_dotnet=false want_frontend=false t
  for t in "$@"; do
    case "$t" in
      analytics)     rust_bins="$rust_bins analytics" ;;
      authenticator) rust_bins="$rust_bins authenticator" ;;
      rust)          rust_bins="$rust_bins analytics authenticator" ;;
      identity)      want_dotnet=true ;;
      frontend)      want_frontend=true ;;
      all)           rust_bins="$rust_bins analytics authenticator"; want_dotnet=true; want_frontend=true ;;
      *) echo "ERROR: unknown target: $t" >&2; cmd_build_help; return 2 ;;
    esac
  done
  rust_bins="$(trim "$rust_bins")"
  # shellcheck disable=SC2086 # word-split the bin list intentionally
  [[ -n "$rust_bins" ]] && build_rust_bins $rust_bins
  [[ "$want_dotnet"   == true ]] && "${compose_cmd[@]}" run --rm build-dotnet
  [[ "$want_frontend" == true ]] && "${compose_cmd[@]}" run --rm build-frontend
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
    --profile auth-fakeidp --profile auth-keycloak \
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

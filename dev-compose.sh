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
COMPOSE_INSTANCE=""

# Who the seed container runs as — see the `user:` key on seed-sample in
# docker-compose.yml. Whoever runs this script owns the checkout the seed
# writes its manifest into, so that is the identity the container needs.
SEED_UID="$(id -u)"
SEED_GID="$(id -g)"
export SEED_UID SEED_GID

# ──────────────────────────────────────────────────────────────────────
# Shared helpers
# ──────────────────────────────────────────────────────────────────────

# bash 3.2 (Mac default) lacks associative arrays. Plain strings + tiny
# helpers keep this script portable.
trim()     { local s="$1"; s="${s#"${s%%[![:space:]]*}"}"; s="${s%"${s##*[![:space:]]}"}"; printf '%s' "$s"; }
contains() { case " $1 " in *" $2 "*) return 0 ;; esac; return 1; }
add()      { local list="$1" item="$2"; contains "$list" "$item" && printf '%s' "$list" || printf '%s %s' "$list" "$item"; }

compose_project_name() {
  local instance="${1:-}"
  if [[ -z "$instance" ]]; then
    printf '%s' "insight"
    return 0
  fi
  if [[ "$instance" == "worktree" ]]; then
    local worktree_root
    worktree_root="$(git rev-parse --show-toplevel 2>/dev/null)" || {
      echo "ERROR: --instance=worktree requires a git worktree." >&2
      return 2
    }
    instance="$(basename "$worktree_root")"
    if [[ "$instance" == "back" ]]; then
      instance="$(basename "$(dirname "$worktree_root")")"
    fi
  fi
  case "$instance" in
    *[!a-z0-9_-]*|[-_]*|"")
      echo "ERROR: --instance must start with a lowercase letter or digit and contain only lowercase letters, digits, hyphens, or underscores." >&2
      return 2
      ;;
  esac
  printf 'insight-%s' "$instance"
}

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

Bring the stack up: build host-side artefacts (Rust + optional
frontend dist), generate a per-run compose override that flips selected
services to ghcr images, then `docker compose up -d`.

Options:
  --from-ghcr=svc1,svc2     Pull these backend services from ghcr instead
                            of building. Recognised:
                            analytics, identity, identity-resolution.
  --watch=svc1,svc2         Run selected Rust services from source with
                            cargo-watch. Recognised: analytics.
  --build-only=svc1,svc2    Build only these; everything else from ghcr.
  --frontend-mode=MODE      Override FRONTEND_MODE for this run.
                            (dev | built | ghcr)
  --authenticator-redirect=URI
                            Register an extra OIDC redirect_uri in the
                            generated Keycloak realm. Repeatable. The two
                            default localhost callbacks are always kept.
  --auth=MODE               Override AUTH_MODE (fakeidp|keycloak) from
                            .env.compose for this run only.
                            (fakeidp | keycloak, default: fakeidp)
  --no-frontend             Don't start any frontend variant.
  --skip-build              Don't rebuild artefacts — reuse what's
                            already in deploy/compose/build/.
  --instance=NAME           Isolate containers, networks, and volumes as
                            insight-NAME. Default: insight.
                            Use worktree to derive NAME from the checkout.
                            Full instances cannot run concurrently because
                            published host ports are shared.
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
  # 0644, not 0600. The authenticator image runs as uid 1000 (its Dockerfile's
  # `appuser`) and bind-mounts this directory read-only, so an owner-only file
  # is unreadable to it whenever the host uid differs — which is every Linux CI
  # runner, where the checkout belongs to uid 1001. Docker Desktop hides this by
  # presenting mounted files as the container user, so it reproduces only in CI
  # and only as "Permission denied (os error 13)" on gear init.
  #
  # Safe because of what this key is: an ephemeral P-256 key generated per
  # checkout into a gitignored directory, used to sign dev tokens for a local
  # stand and nothing else. A deployment mounts a real key from a K8s Secret.
  # The service-token PRIVATE half below stays 0600 — the authenticator resolves
  # only the named `*.pub.pem` entries in `public_key_paths` and never reads it.
  chmod 644 "$key"
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
  # `-new` is REQUIRED, not decoration. OpenSSL 1.1+/3.x implies it when -x509
  # and -key are both given; LibreSSL (the macOS system openssl) does not, and
  # instead tries to READ a certificate request from stdin — failing with
  # "unable to load X509 request ... Expecting: CERTIFICATE REQUEST" and
  # producing no cert. Passing it explicitly is correct on both.
  # Errors are NOT swallowed here: without this cert the authn-tls discovery
  # front cannot start and analytics can never verify a gateway JWT, so a
  # silent failure surfaces much later as an unexplained auth failure.
  if ! openssl req -new -x509 -key "$dir/server.key" -out "$cert" -days 3650 -config "$cnf"; then
    echo "ERROR: could not generate the authn-tls certificate ($cert)." >&2
    return 1
  fi
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

# The `volumes:` a ghcr'd service keeps, as a YAML block.
#
# The flip must drop exactly ONE mount — the host-built binary under
# deploy/compose/build/. A published image already carries it, and leaving the
# mount shadows the image with a file this run never built (or, with nothing
# built, with a directory compose invents and container init rejects).
#
# Every OTHER mount has to survive, which a blanket `volumes: !override []` did
# not. They are the dev stack's own configuration and no image can carry them:
# the *-fullauth.yaml that turns ON real gateway-JWT verification (over the
# committed placeholder config the image bakes), the self-signed CA for the
# authn-tls discovery front, the per-run authenticator signing key, and the
# compose route table. Dropping them leaves a service on the placeholder config
# — an auth-disabled stand, which is precisely what this stand exists to
# disprove, and it would have looked like a pass.
#
# Compose replaces the whole list rather than subtracting from it, so the
# survivors have to be restated. They are READ BACK from docker-compose.yml
# rather than listed here: a hand-kept copy is a second source of truth, and it
# was already wrong once — identity-resolution's /certs mount was missing from
# it, so the service died on "failed to read custom CA certificate bundle" and
# every login 500'd.
ghcr_volumes_block() {
  local svc="$1" mounts
  mounts="$(ghcr_kept_mounts "$svc")" || return 1
  echo "    volumes: !override"
  [[ -n "$mounts" ]] && printf '      - %s\n' $mounts
  return 0
}

# Every mount docker-compose.yml declares for `svc` except the host-built
# binary, as `source:target[:mode]` relative to the repo root.
ghcr_kept_mounts() {
  local svc="$1" out
  out="$(docker compose -f docker-compose.yml --profile auth-keycloak --profile auth-fakeidp \
           config --format json 2>/dev/null |
    SERVICE="$svc" python3 -c '
import json, os, sys

root = os.getcwd().rstrip("/") + "/"
service = json.load(sys.stdin)["services"][os.environ["SERVICE"]]

for volume in service.get("volumes") or []:
    source = volume.get("source", "")
    # The one mount the published image replaces.
    if source.startswith(root + "deploy/compose/build/"):
        continue
    mode = ":ro" if volume.get("read_only") else ""
    target = volume["target"]
    print("./" + source[len(root):] + ":" + target + mode)
')" || { echo "ERROR: cannot read $svc mounts from docker-compose.yml" >&2; return 1; }
  printf '%s' "$out"
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
  local instance="$COMPOSE_INSTANCE"
  # Repeatable. Empty => gen-realm.py keeps its own defaults untouched.
  local authenticator_redirects=""
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
      --authenticator-redirect=*)
        authenticator_redirects="$(add "$authenticator_redirects" "${1#*=}")"; shift ;;
      --authenticator-redirect)
        [[ $# -ge 2 ]] || { echo "ERROR: --authenticator-redirect requires a value." >&2; return 2; }
        authenticator_redirects="$(add "$authenticator_redirects" "$2")"; shift 2 ;;
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
  COMPOSE_PROJECT_NAME="$(compose_project_name "$instance")" || return $?
  export COMPOSE_PROJECT_NAME
  [[ -n "$instance" ]] && echo "Compose instance → $COMPOSE_PROJECT_NAME"
  local instance_mariadb_fresh=false
  local instance_clickhouse_fresh=false
  if [[ -n "$instance" ]]; then
    if [[ "${MARIADB_EXTERNAL:-false}" != "true" ]] &&
       ! docker volume inspect "${COMPOSE_PROJECT_NAME}_mariadb-data" >/dev/null 2>&1; then
      instance_mariadb_fresh=true
    fi
    if [[ "${CLICKHOUSE_EXTERNAL:-false}" != "true" ]] &&
       ! docker volume inspect "${COMPOSE_PROJECT_NAME}_clickhouse-data" >/dev/null 2>&1; then
      instance_clickhouse_fresh=true
    fi
  fi

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
  # The seed-sample container reads AUTH_MODE too (deploy/seed/profiles.py's
  # get_login_id_pairs) to pick which roster personas get a login-id fixture —
  # export so the child `docker compose` process's env-var interpolation sees it.
  export AUTH_MODE

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
  local all_backend="analytics identity-resolution authenticator gateway"
  local watchable_services="analytics"
  local ghcr_list=""
  local watch_list=""
  local build_list=""

  [[ -n "${ANALYTICS_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" analytics)
  [[ -n "${IDENTITY_RESOLUTION_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" identity-resolution)
  [[ -n "${AUTHENTICATOR_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" authenticator)
  [[ -n "${GATEWAY_IMAGE:-}" ]] && ghcr_list=$(add "$ghcr_list" gateway)

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
  contains "$ghcr_list" identity-resolution && [[ -z "${IDENTITY_RESOLUTION_IMAGE:-}" ]] && export IDENTITY_RESOLUTION_IMAGE="ghcr.io/constructorfabric/insight-identity-resolution:${IDENTITY_RESOLUTION_GHCR_TAG:-latest}"
  contains "$ghcr_list" authenticator && [[ -z "${AUTHENTICATOR_IMAGE:-}" ]] && export AUTHENTICATOR_IMAGE="ghcr.io/constructorfabric/insight-authenticator:${AUTHENTICATOR_GHCR_TAG:-latest}"
  contains "$ghcr_list" gateway && [[ -z "${GATEWAY_IMAGE:-}" ]] && export GATEWAY_IMAGE="ghcr.io/constructorfabric/insight-gateway:${GATEWAY_GHCR_TAG:-latest}"
  true

  # ── Generate per-run override ────────────────────────────────────
  # (see ghcr_volumes_block below for what the flip keeps and drops)
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
          #
          # `command: !reset null` falls back to the image's own CMD, which is
          # the same `-c /app/config/insight.yaml` invocation minus the
          # watched-path wrapper — so the config mount below is what it reads.
          cat <<YML
  ${svc}:
    build: !reset null
    entrypoint: !reset null
    command: !reset null
    platform: linux/amd64
$(ghcr_volumes_block "$svc")
YML
          if [[ "$svc" == "identity-resolution" ]]; then
            # The one-shot migrate companion must flip to the ghcr image too:
            # left alone it keeps the build + local-binary bind mount (which
            # was intentionally not built in ghcr mode), never starts, and the
            # server blocks forever on service_completed_successfully. The
            # base command (…migrate) is kept — the image's CMD has no
            # subcommand, so resetting it here would start a SERVER that never
            # completes and the dependents would wait forever.
            cat <<YML
  identity-resolution-migrate:
    build: !reset null
    platform: linux/amd64
$(ghcr_volumes_block identity-resolution-migrate)
YML
          fi
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
    # gen-realm.py's own --authenticator-redirect REPLACES its defaults rather
    # than appending, so whenever we pass any URI we must re-state the two
    # defaults too — dropping them would deregister the human login origins
    # and break `./dev-compose.sh up --auth keycloak`.
    local redirect_args=""
    if [[ -n "$authenticator_redirects" ]]; then
      local _uri
      for _uri in $authenticator_redirects \
                  "http://localhost:3000/auth/callback" \
                  "http://localhost:8080/auth/callback"; do
        redirect_args="$redirect_args --authenticator-redirect $_uri"
      done
      echo "    registering redirect URIs:$redirect_args"
    fi
    # shellcheck disable=SC2086  # redirect_args is a deliberately word-split flag list
    python3 deploy/compose/keycloak/gen-realm.py \
      --dev-email "$dev_lead_email" \
      $redirect_args \
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
    # The login-bootstrap resolve is scoped to idp.source_type; keycloak's
    # sub differs in KIND from fakeidp's (gen-realm.py sets each realm user's
    # id to their OWN roster uuid, so sub IS that uuid — not the fixed
    # "fakeidp|dev" string fakeidp issues), so it must be seeded/looked-up
    # under its own source_type, not the fakeidp default (see
    # deploy/seed/profiles.py::get_login_id_pairs).
    export AUTHENTICATOR_IDP_SOURCE_TYPE="keycloak"
    echo "keycloak issuer → ${kc_base}/realms/insight (host IP; browser + authenticator reachable)"

    # AUTH_DISABLED is a separate, blunter bypass; if it's on, real login is
    # still skipped regardless.
    [[ "${AUTH_DISABLED:-false}" == "true" ]] && {  # RULE-DEFAULTS-OK: purely a cosmetic warn-or-not check, not a config value
      echo "WARN: AUTH_DISABLED=true forces an auth bypass — unset it to" >&2
      echo "      exercise the real Keycloak login flow." >&2
    }
  fi

  local compose_cmd=(docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$env_file" -f docker-compose.yml -f "$override")
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
    # Each variant listens on its own port, and the gateway has to be told
    # which. Getting it wrong is a confusing failure: the container is up and
    # its name resolves, so the gateway reports "connect() failed (111:
    # Connection refused)" rather than anything about a port.
    if [[ -z "${FRONTEND_INTERNAL_PORT:-}" ]]; then
      case "$FRONTEND_MODE" in
        dev)   FRONTEND_INTERNAL_PORT=5173 ;;  # vite
        built) FRONTEND_INTERNAL_PORT=80   ;;  # stock nginx image, runs as root
        ghcr)  FRONTEND_INTERNAL_PORT=8080 ;;  # published image, runs as uid 101
      esac
      export FRONTEND_INTERNAL_PORT
    fi
  fi
  profiles+=(--profile "auth-$AUTH_MODE")

  # ── Build phase ──────────────────────────────────────────────────
  if [[ "$skip_build" != "true" ]]; then
    echo "=== Building artefacts (skip with --skip-build) ==="
    if [[ "$AUTH_MODE" == fakeidp ]]; then
      echo "--- Image: fakeidp"
      "${compose_cmd[@]}" --profile auth-fakeidp build fakeidp
    fi
    # A service's binary is bind-mounted as a FILE, so omitting it from the
    # build while it still has that mount makes compose auto-create the mount
    # source as an empty directory and container init fails. Every service left
    # out here must therefore be one the ghcr override took the mount off.
    local rust_bins=""
    contains "$ghcr_list" authenticator || rust_bins="authenticator"
    contains "$ghcr_list" analytics || contains "$watch_list" analytics || rust_bins="$rust_bins analytics"
    contains "$ghcr_list" identity-resolution || rust_bins="$rust_bins identity-resolution"
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
            protobuf-compiler libprotobuf-dev pkg-config libssl-dev cmake > /dev/null
          cargo build --release$bin_flags
          mkdir -p /out/analytics /out/authenticator /out/identity-resolution
          # Publish with cat + cmp, NOT cp or install. /out is a macOS bind
          # mount; cp there fails with \"error deallocating ...: Invalid
          # argument\" AFTER writing a SHORT file (observed 34787328 of
          # 49532680 bytes for analytics) and install aborts outright. Worse,
          # \`cp X Y && chmod Y\` hides the failure from set -e entirely --
          # bash exempts every command in an && list but the last -- so the
          # build stayed green and shipped a truncated binary that segfaults
          # the instant it is exec'd. cmp makes a bad copy fatal, here.
          if [ -f /target/release/analytics ]; then
            rm -rf /out/analytics/analytics
            cat /target/release/analytics > /out/analytics/analytics
            chmod 0755 /out/analytics/analytics
            cmp -s /target/release/analytics /out/analytics/analytics || { echo 'ERROR: /out/analytics/analytics copied corrupt' >&2; exit 1; }
          fi
          if [ -f /target/release/authenticator ]; then
            rm -rf /out/authenticator/authenticator
            cat /target/release/authenticator > /out/authenticator/authenticator
            chmod 0755 /out/authenticator/authenticator
            cmp -s /target/release/authenticator /out/authenticator/authenticator || { echo 'ERROR: /out/authenticator/authenticator copied corrupt' >&2; exit 1; }
          fi
          if [ -f /target/release/identity-resolution ]; then
            rm -rf /out/identity-resolution/identity-resolution
            cat /target/release/identity-resolution > /out/identity-resolution/identity-resolution
            chmod 0755 /out/identity-resolution/identity-resolution
            cmp -s /target/release/identity-resolution /out/identity-resolution/identity-resolution || { echo 'ERROR: /out/identity-resolution/identity-resolution copied corrupt' >&2; exit 1; }
          fi
        "
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
  if ! "${compose_cmd[@]}" ${profiles[@]+"${profiles[@]}"} up -d --remove-orphans; then
    "${compose_cmd[@]}" ps -a >&2 || true
    "${compose_cmd[@]}" logs --no-color --tail=80 >&2 || true
    if [[ -n "$instance" &&
          ( "$instance_mariadb_fresh" == "true" || "$instance_clickhouse_fresh" == "true" ) ]]; then
      echo "ERROR: stack startup failed; removing newly created database state." >&2
      "${compose_cmd[@]}" ${profiles[@]+"${profiles[@]}"} down --remove-orphans >/dev/null 2>&1 || true
      if [[ "$instance_mariadb_fresh" == "true" ]]; then
        docker volume rm "${COMPOSE_PROJECT_NAME}_mariadb-data" >/dev/null 2>&1 || true
      fi
      if [[ "$instance_clickhouse_fresh" == "true" ]]; then
        docker volume rm "${COMPOSE_PROJECT_NAME}_clickhouse-data" >/dev/null 2>&1 || true
        docker volume rm "${COMPOSE_PROJECT_NAME}_clickhouse-logs" >/dev/null 2>&1 || true
      fi
    else
      echo "ERROR: stack startup failed." >&2
    fi
    return 1
  fi

  echo
  "${compose_cmd[@]}" ps
  echo

  # ── First-run auto-seed ─────────────────────────────────────────────
  # Run seed once on the first up after the wizard. The SEEDED_LOCAL_*
  # markers in .env.compose are flipped to true on success so subsequent
  # `up` calls skip this block. For external DBs, the wizard pre-marks
  # them seeded unless the user explicitly opted in.
  local need_maria=false need_ch=false
  if [[ -n "$instance" ]]; then
    need_maria="$instance_mariadb_fresh"
    need_ch="$instance_clickhouse_fresh"
  else
    [[ "${SEEDED_LOCAL_MARIA:-}" != "true" ]] && need_maria=true
    [[ "${SEEDED_LOCAL_CH:-}"    != "true" ]] && need_ch=true
  fi
  if [[ "$need_maria" == "true" || "$need_ch" == "true" ]]; then
    local seed_target=""
    if   [[ "$need_maria" == "true" && "$need_ch" == "true" ]]; then seed_target=all
    elif [[ "$need_maria" == "true" ]]; then                          seed_target=identity
    else                                                              seed_target=silver
    fi
    echo "=== First-run seed ($seed_target) ==="
    if cmd_seed --env-file "$env_file" "$seed_target"; then
      if [[ -z "$instance" ]]; then
        [[ "$need_maria" == "true" ]] && update_env_var "$env_file" SEEDED_LOCAL_MARIA true
        [[ "$need_ch"    == "true" ]] && update_env_var "$env_file" SEEDED_LOCAL_CH    true
      fi
    else
      echo "WARN: seed failed; SEEDED_LOCAL_* not updated." >&2
      if [[ -n "$instance" ]]; then
        echo "      Re-run: ./dev-compose.sh seed --instance=$instance $seed_target" >&2
      else
        echo "      Re-run: ./dev-compose.sh seed $seed_target" >&2
      fi
    fi
    echo
  fi

  local frontend_up=true
  [[ "$no_frontend" == "true" ]] && frontend_up=false
  report_service_urls "$frontend_up" "$AUTH_MODE"
  echo

  local instance_option=""
  [[ -n "$instance" ]] && instance_option=" --instance=$instance"
  echo "Service URLs: ./dev-compose.sh urls"
  echo "Stop:        ./dev-compose.sh down$instance_option"
  echo "Rebuild one: ./dev-compose.sh build$instance_option <service>"
  echo "Re-seed:     ./dev-compose.sh seed$instance_option"
  echo "Wipe state:  ./dev-compose.sh prune$instance_option"
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
  printf '  %-18s %s\n' "Identity API"    "http://$h:${IDENTITY_RESOLUTION_PORT:-8086}"
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
  --volumes  / -v  Also remove named volumes. For the default instance,
                   also wipe deploy/compose/build/.
  --instance=NAME   Target the isolated insight-NAME stack. Default: insight.
  --env-file=PATH  Alternate dotenv file.
EOF
}

cmd_down() {
  local env_file=".env.compose"
  local instance="$COMPOSE_INSTANCE"
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
  COMPOSE_PROJECT_NAME="$(compose_project_name "$instance")" || return $?
  export COMPOSE_PROJECT_NAME

  local override="deploy/compose/override.generated.yml"
  local compose_cmd=(docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  # EVERY profile, including the datastores. Compose only acts on services in
  # the active profile set, so omitting local-mariadb/local-clickhouse left
  # both containers running and — with --volumes — left `mariadb-data` and
  # `clickhouse-data` behind, silently carrying one run's data into the next.
  # That is the opposite of the documented contract below and of the "reset by
  # volume teardown, never TRUNCATE" rule the test stand depends on. Listing
  # them is safe for external-DB setups: an inactive service is a no-op here.
  "${compose_cmd[@]}" \
    --profile local-mariadb --profile local-clickhouse \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile auth-fakeidp --profile auth-keycloak \
    --profile build --profile seed \
    --profile local-mariadb --profile local-clickhouse \
    down $([[ "$wipe" == "true" ]] && echo "--volumes --remove-orphans")

  if [[ "$wipe" == "true" && -z "$instance" ]]; then
    echo "Wiping host-side build artefacts (deploy/compose/build/)..."
    # The build container writes these as root, so on Linux (every CI runner)
    # they belong to a uid this shell is not — `rm` then fails on each binary
    # and, under `set -e`, takes the whole teardown down with it. That turned a
    # run whose tests all PASSED into a red job.
    #
    # Best-effort: nothing here is state the next run depends on, since a fresh
    # `up` rebuilds or re-pulls. What is left behind is reported rather than
    # fatal.
    rm -rf deploy/compose/build/ 2>/dev/null || {
      echo "NOTE: some build artefacts could not be removed (root-owned — the" >&2
      echo "      build container wrote them). They are rebuilt on the next up:" >&2
      find deploy/compose/build -type f 2>/dev/null | sed 's/^/        /' >&2 || true
    }
  elif [[ "$wipe" == "true" ]]; then
    echo "Preserving worktree build artefacts."
  fi
  echo "Done."
}

# ──────────────────────────────────────────────────────────────────────
# build
# ──────────────────────────────────────────────────────────────────────

cmd_build_help() {
  cat <<'EOF'
usage: dev-compose.sh build [--instance NAME] [--env-file PATH] <target>

Rebuild one host-side artefact and let the already-running container
pick it up via ENABLE_AUTO_RELOAD.

Targets:
  analytics            Rust analytics binary only.
  authenticator        Rust authenticator binary only.
  identity-resolution  Rust identity-resolution binary only.
  frontend             pnpm build → dist/.
  rust                 All Rust services.
  all                  Everything (Rust + frontend).
EOF
}

cmd_build() {
  local env_file=".env.compose"
  local instance="$COMPOSE_INSTANCE"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*) env_file="${1#*=}"; shift ;;
      --env-file)   env_file="$2"; shift 2 ;;
      *) break ;;
    esac
  done

  local target="${1:-}"
  [[ -z "$target" || "$target" == "-h" || "$target" == "--help" ]] && { cmd_build_help; return 0; }

  env_file="$(resolve_env_file "$env_file")"
  set -a; source "$env_file"; set +a
  COMPOSE_PROJECT_NAME="$(compose_project_name "$instance")" || return $?
  export COMPOSE_PROJECT_NAME

  local compose_cmd=(docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$env_file" -f docker-compose.yml --profile build)
  build_rust_bins() {
    local bin_flags=""
    local b
    for b in "$@"; do bin_flags="$bin_flags --bin $b"; done
    "${compose_cmd[@]}" run --rm build-rust bash -c "
      set -eux
      apt-get update && apt-get install -y --no-install-recommends \
        protobuf-compiler libprotobuf-dev pkg-config libssl-dev cmake > /dev/null
      cargo build --release$bin_flags
      mkdir -p /out/analytics /out/authenticator /out/identity-resolution
      # cat + cmp, not cp/install -- see the identical block in cmd_up for why
      # a plain cp here silently ships a truncated, instantly-segfaulting
      # binary.
      if [ -f /target/release/analytics ]; then
        rm -rf /out/analytics/analytics
        cat /target/release/analytics > /out/analytics/analytics
        chmod 0755 /out/analytics/analytics
        cmp -s /target/release/analytics /out/analytics/analytics || { echo 'ERROR: /out/analytics/analytics copied corrupt' >&2; exit 1; }
      fi
      if [ -f /target/release/authenticator ]; then
        rm -rf /out/authenticator/authenticator
        cat /target/release/authenticator > /out/authenticator/authenticator
        chmod 0755 /out/authenticator/authenticator
        cmp -s /target/release/authenticator /out/authenticator/authenticator || { echo 'ERROR: /out/authenticator/authenticator copied corrupt' >&2; exit 1; }
      fi
      if [ -f /target/release/identity-resolution ]; then
        rm -rf /out/identity-resolution/identity-resolution
        cat /target/release/identity-resolution > /out/identity-resolution/identity-resolution
        chmod 0755 /out/identity-resolution/identity-resolution
        cmp -s /target/release/identity-resolution /out/identity-resolution/identity-resolution || { echo 'ERROR: /out/identity-resolution/identity-resolution copied corrupt' >&2; exit 1; }
      fi
    "
  }

  # Accept MULTIPLE targets, e.g. `build authenticator identity-resolution`.
  # Rust bins are batched into one build; frontend runs once if requested.
  local rust_bins="" want_frontend=false t
  for t in "$@"; do
    case "$t" in
      analytics)           rust_bins="$rust_bins analytics" ;;
      authenticator)       rust_bins="$rust_bins authenticator" ;;
      identity-resolution) rust_bins="$rust_bins identity-resolution" ;;
      rust)                rust_bins="$rust_bins analytics authenticator identity-resolution" ;;
      frontend)            want_frontend=true ;;
      all)                 rust_bins="$rust_bins analytics authenticator identity-resolution"; want_frontend=true ;;
      *) echo "ERROR: unknown target: $t" >&2; cmd_build_help; return 2 ;;
    esac
  done
  rust_bins="$(trim "$rust_bins")"
  # shellcheck disable=SC2086 # word-split the bin list intentionally
  [[ -n "$rust_bins" ]] && build_rust_bins $rust_bins
  [[ "$want_frontend" == true ]] && "${compose_cmd[@]}" run --rm build-frontend
  echo "Done. If a runtime container has ENABLE_AUTO_RELOAD=true it will restart automatically."
}

# ──────────────────────────────────────────────────────────────────────
# seed
# ──────────────────────────────────────────────────────────────────────

cmd_seed_help() {
  cat <<'EOF'
usage: dev-compose.sh seed [--instance NAME] [--env-file PATH] [identity|silver|all]

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
  local instance="$COMPOSE_INSTANCE"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --env-file=*) env_file="${1#*=}"; shift ;;
      --env-file)   env_file="$2"; shift 2 ;;
      *) break ;;
    esac
  done
  if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then cmd_seed_help; return 0; fi

  env_file="$(resolve_env_file "$env_file")"
  COMPOSE_PROJECT_NAME="$(compose_project_name "$instance")" || return $?
  export COMPOSE_PROJECT_NAME
  local override="deploy/compose/override.generated.yml"
  local compose_cmd=(docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$env_file" -f docker-compose.yml)
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
usage: dev-compose.sh prune [--instance NAME]

DESTRUCTIVE — wipes local stack state. Interactive: you must approve
each step. There is no `--yes` switch on purpose.

With --instance, only that instance's containers, networks, and named
volumes are removed. Worktree-level config, keys, and build artefacts
are preserved.

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
  local instance="$COMPOSE_INSTANCE"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -h|--help) cmd_prune_help; return 0 ;;
      *) echo "ERROR: unknown arg: $1" >&2; cmd_prune_help; return 2 ;;
    esac
  done
  COMPOSE_PROJECT_NAME="$(compose_project_name "$instance")" || return $?
  export COMPOSE_PROJECT_NAME

  if [[ -n "$instance" ]]; then
    cat <<EOF
This will permanently remove Docker state for Compose instance
$COMPOSE_PROJECT_NAME:
  • containers
  • named volumes
  • the instance network

Worktree-level build artefacts, generated config, keys, and .env.compose
will be preserved.

EOF
  else
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
  fi
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
  local compose_cmd=(docker compose --project-name "$COMPOSE_PROJECT_NAME" --env-file "$env_file" -f docker-compose.yml)
  [[ -f "$override" ]] && compose_cmd+=(-f "$override")

  echo "=== docker compose down --volumes --remove-orphans ==="
  "${compose_cmd[@]}" \
    --profile front-dev --profile front-built --profile front-ghcr \
    --profile auth-fakeidp --profile auth-keycloak \
    --profile build --profile seed \
    --profile local-mariadb --profile local-clickhouse \
    down --volumes --remove-orphans || true

  if [[ -z "$instance" && -d deploy/compose/build ]]; then
    echo "Removing deploy/compose/build/..."
    rm -rf deploy/compose/build/
  fi
  if [[ -z "$instance" && -d deploy/compose/authenticator-dev-keys ]]; then
    echo "Removing deploy/compose/authenticator-dev-keys/ (dev signing key)..."
    rm -rf deploy/compose/authenticator-dev-keys/
  fi
  if [[ -z "$instance" && -f "$override" ]]; then
    echo "Removing $override..."
    rm -f "$override"
  fi
  if [[ -z "$instance" && -f .env.compose ]]; then
    echo "Removing .env.compose..."
    rm -f .env.compose
  fi

  echo
  echo "Stack state wiped."
  echo

  if [[ -n "$instance" ]]; then
    echo "Done."
    return
  fi

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

# ─── test-stand ────────────────────────────────────────────────────────
#
# The automated-test face of the same stack. Every verb DELEGATES to the
# cmd_* functions above — this block owns policy (which knobs are forced,
# when the stand is considered ready), never mechanism.
#
# It is deliberately isolated from the developer's own `.env.compose`:
# everything runs through a generated, gitignored `.env.compose.test-stand`,
# so `./dev-compose.sh up` keeps behaving exactly as it did.

TEST_STAND_ENV_FILE=".env.compose.test-stand"
# The origin the app is driven at, by a browser runner and by a human alike.
#
# Two things are load-bearing here.
#
# It is the GATEWAY, not the `insight-front` alias. The front container's nginx
# proxies only `/api/`; `/auth/login` there falls through to `try_files …
# /index.html` and serves the SPA, so the OIDC chain never starts (verified
# in-network: `insight-front/auth/login` -> 200 HTML, `gateway:8080/auth/login`
# -> 302 to Keycloak). The gateway fronts the SPA, `/auth/*` and `/api/*`
# together, which is the topology the published SPA is built for.
#
# And the host is `localhost`, not the `gateway` service name, because the
# session cookie is `__Host-`-prefixed: browsers only accept it from a
# trustworthy origin, and `localhost` is trustworthy over plain http while
# `gateway:8080` is not. Chromium's --unsafely-treat-insecure-origin-as-secure
# does NOT lift that (measured on Chromium 149: window.isSecureContext stays
# false with the flag, in every launch mode), so an in-network browser runner
# keeps the `localhost` NAME and points it at the gateway container with
# --host-resolver-rules instead. No host port is involved for that runner.
# Read from the generated env file rather than baked in at file scope, so a
# stand on a non-default GATEWAY_PORT stays consistent.
test_stand_origin() {
  local port
  port="$(grep -E '^[[:space:]]*GATEWAY_PORT=' "$TEST_STAND_ENV_FILE" 2>/dev/null | tail -1 | cut -d= -f2)"
  printf 'http://localhost:%s' "${port:-8080}"
}
TEST_STAND_READY_TIMEOUT=240
TEST_STAND_READY_INTERVAL=5

cmd_test_stand_help() {
  cat <<'EOF'
usage: dev-compose.sh test-stand <up|seed|test|down> [args]

The stack in test configuration: pinned ghcr images for the frontend and all
four backend services, real Keycloak login, and a readiness gate that waits
for dbt-built gold data rather than for containers to report healthy.

  up      Generate .env.compose.test-stand, bring the stack up, seed it, and
          block until EVERY gold observation table the seed populates proves
          dbt rebuilt it for this run and left a positive observation in it.

          The four backend services (analytics, authenticator,
          identity-resolution, gateway) and the frontend are PULLED, each
          pinned to its own chart's appVersion — never :latest, and never
          compiled here. Building them took ~26 minutes for code the stand
          does not change.

          --build-backend  Compile the backend from this working tree instead.
                           Needed to test a backend change: `up` otherwise
                           refuses when the tree differs from origin/main
                           under src/backend/, since the pinned images would
                           not be what ran.
  seed    Re-seed the running stand (default target: all).
  test    Run the stand suite against an already-up stand. Passes extra
          arguments through to pytest — no `--` separator.
          The suite aims itself at this stand; override with pytest's own
          --base-url <url> and --stand-manifest <path> when pointing it
          somewhere else.

          --image <ref>  Run inside an already-pulled ui-tests image instead
                         of on the host, sharing the gateway's network
                         namespace. Never builds: pull the image first. Test
                         paths are then IMAGE-SIDE (/tests/stand/ui, not
                         tests/stand/ui), and pytest-playwright's artefacts
                         land in ./test-results as usual.
  down    Stop the stand and REMOVE its volumes, so the next `up` starts
          from empty databases.

Isolation: reads and writes .env.compose.test-stand only — never your own
.env.compose. Airbyte and Argo are never started.
EOF
}

# Resolve an image from its own chart's appVersion, so the stand runs the same
# build the umbrella chart would deploy. Never `:latest`, which would make a run
# unreproducible — build-images.yml's bump-descriptors writes these on every
# successful main push, so an appVersion names an image that provably exists.
test_stand_pinned_image() {
  local chart="$1" name="$2" version
  [[ -f "$chart" ]] || { echo "ERROR: $chart not found — cannot pin $name." >&2; return 1; }
  version="$(awk -F'"' '/^appVersion:/ {print $2; exit}' "$chart")"
  [[ -n "$version" ]] || { echo "ERROR: no appVersion in $chart — cannot pin $name." >&2; return 1; }
  printf 'ghcr.io/constructorfabric/insight-%s:%s' "$name" "$version"
}

test_stand_frontend_image() {
  test_stand_pinned_image src/frontend/helm/Chart.yaml front
}

# The backend services the stand runs from published images rather than from
# source, as "<compose env var>|<chart path>|<image name>".
#
# Building these four compiles the Rust workspace twice — once on the host for
# the bind-mounted binaries, then again inside each service image — which is
# where the stand's wall-clock went.
TEST_STAND_PINNED_BACKENDS=(
  "ANALYTICS_IMAGE|src/backend/services/analytics/helm/Chart.yaml|analytics"
  "AUTHENTICATOR_IMAGE|src/backend/services/authenticator/helm/Chart.yaml|authenticator"
  "IDENTITY_RESOLUTION_IMAGE|src/backend/services/identity-resolution/helm/Chart.yaml|identity-resolution"
  "GATEWAY_IMAGE|src/backend/services/gateway/helm/Chart.yaml|gateway"
)

# Pin and pull every backend image, or fail.
#
# Fails rather than falling back to a build on purpose: a silent fallback is how
# a 26-minute compile comes back invisibly.
test_stand_pull_backends() {
  local entry var chart name image
  echo "=== Pinning the backend to published images (skip with --build-backend) ==="
  for entry in "${TEST_STAND_PINNED_BACKENDS[@]}"; do
    IFS='|' read -r var chart name <<<"$entry"
    image="$(test_stand_pinned_image "$chart" "$name")" || return 1
    echo "    ${name}: ${image}"
    docker pull --quiet "$image" >/dev/null || {
      echo "ERROR: cannot pull $image (pinned by $chart's appVersion)." >&2
      echo "       Not falling back to a source build — that would report a pass" >&2
      echo "       for an image this run never ran. Check ghcr access, or pass" >&2
      echo "       --build-backend to build from source deliberately." >&2
      return 1; }
    update_env_var "$TEST_STAND_ENV_FILE" "$var" "$image"
  done
}

# Refuse to pin when the working tree's backend differs from what the charts
# describe.
#
# The appVersions track main. A branch that edits src/backend/** and then runs
# against published images would report green for code it never executed. The PR
# path filter makes this unreachable in the normal lane; this covers
# workflow_dispatch and local runs, which bypass it.
test_stand_backend_matches_charts() {
  git rev-parse --git-dir >/dev/null 2>&1 || return 0
  git remote get-url origin >/dev/null 2>&1 || return 0
  git rev-parse --verify --quiet origin/main >/dev/null || return 0

  local changed
  changed="$(git diff --name-only origin/main -- src/backend 2>/dev/null | head -5)"
  [[ -z "$changed" ]] && return 0

  echo "ERROR: this tree changes src/backend/ relative to origin/main:" >&2
  printf '         %s\n' $changed >&2
  echo "       The stand pins each backend image to its chart's appVersion, which" >&2
  echo "       tracks main — so those changes would NOT be what runs. Pass" >&2
  echo "       --build-backend to build this tree instead." >&2
  return 1
}

# Derive the test env file from the committed example, overriding only the
# knobs the test path forces. SEEDED_LOCAL_* are blanked so every `up` seeds.
test_stand_write_env() {
  local image="$1" auth_mode="$2"
  [[ -f .env.compose.example ]] || { echo "ERROR: .env.compose.example not found." >&2; return 1; }
  cp .env.compose.example "$TEST_STAND_ENV_FILE"
  update_env_var "$TEST_STAND_ENV_FILE" FRONTEND_MODE   "ghcr"
  update_env_var "$TEST_STAND_ENV_FILE" FRONTEND_IMAGE  "$image"
  update_env_var "$TEST_STAND_ENV_FILE" AUTH_MODE       "$auth_mode"
  update_env_var "$TEST_STAND_ENV_FILE" SEEDED_LOCAL_MARIA ""
  update_env_var "$TEST_STAND_ENV_FILE" SEEDED_LOCAL_CH    ""
  if [[ "$auth_mode" == keycloak ]]; then
    # Point the authenticator at the same origin the realm registers and the
    # browser runner drives, so the callback lands where the session cookie
    # can be set. Left at its .env.compose.example default
    # (http://localhost:3000/auth/callback) the IdP would send an in-network
    # browser to its OWN loopback, and a host client to an origin that serves
    # the SPA rather than the authenticator.
    update_env_var "$TEST_STAND_ENV_FILE" AUTHENTICATOR_REDIRECT_URI "$(test_stand_origin)/auth/callback"
  fi
  echo "=== test-stand env → $TEST_STAND_ENV_FILE (frontend: $image, auth: $auth_mode) ==="
  if [[ "$auth_mode" == keycloak ]]; then
    echo "    app origin: $(test_stand_origin)  callback: $(test_stand_origin)/auth/callback"
  fi
}

# Every gold observation table this seed is expected to populate. The gate
# requires ALL of them: one table proves that dbt ran, not that the seed
# produced the data a test will look for, and the two failures are told apart
# by nobody once the suite starts failing on missing rows.
#
# The list is committed rather than derived. It was read off the evidence
# models' own sources (src/ingestion/gold/<family>_metric_evidence.sql) against
# what deploy/seed/generators/ writes:
#
#   task    <- task_issue_state / task_status_spans / task_worklog_flow  (task.py)
#   git     <- class_git_{commits,file_changes,pull_requests,…}          (git.py)
#   collab  <- class_collab_{chat,email,meeting}_activity, focus_metrics (collab.py)
#   ai      <- class_ai_{assistant,dev}_usage                            (ai.py)
#
# `wiki_metric_observations` is absent ON PURPOSE: its evidence model reads
# class_wiki_* and there is no wiki generator, so requiring it would hang every
# run. The crm, support, hr and people generators have no observation table of
# their own — they feed other surfaces — so they cannot be gated on here.
TEST_STAND_READY_TABLES=(
  task_metric_observations
  git_metric_observations
  collab_metric_observations
  ai_metric_observations
)

test_stand_ch_query() {
  local ch_port="${CLICKHOUSE_HTTP_PORT:-8123}"
  local ch_user="${CLICKHOUSE_USER:-insight}" ch_pass="${CLICKHOUSE_PASSWORD:-insight-local}"
  trim "$(curl -sf -u "${ch_user}:${ch_pass}" --data-binary "$1" \
          "http://localhost:${ch_port}/" 2>/dev/null || true)"
}

# 0 when `table` was rebuilt during this run AND carries a positive observation.
# Otherwise non-zero, echoing which half failed.
#
# Asserted without naming a measure_key. A specific key would have to be kept in
# step with the measure catalogue, and would make the gate pass while every OTHER
# measure in the table was empty — `countIf(value > 0)` asks the question the
# gate actually has, which is whether this family produced any signal at all.
test_stand_table_ready() {
  local table="$1" run_started_at="$2"
  # Run scoping. The obvious filter — `observed_at >= run_started_at` — cannot
  # work: the gold model projects observed_at as a literal
  # CAST(NULL AS Nullable(DateTime64(3))), so it is NULL on every row and the
  # predicate never matches. Instead ask ClickHouse when the table was last
  # written: max(modification_time) over its active parts is real ingestion
  # metadata, and it answers exactly the question the gate cares about —
  # "did dbt rebuild this during THIS run, or am I looking at a previous
  # run's rows?"
  local rebuilt populated
  rebuilt="$(test_stand_ch_query "SELECT max(modification_time) >= toDateTime('${run_started_at}') FROM system.parts WHERE database = 'insight' AND table = '${table}' AND active")"
  if [[ "$rebuilt" != "1" ]]; then
    echo "not rebuilt since ${run_started_at} (got '${rebuilt:-<no response>}', want 1)"
    return 1
  fi

  populated="$(test_stand_ch_query "SELECT countIf(value > 0) > 0 FROM insight.${table}")"
  if [[ "$populated" != "1" ]]; then
    echo "rebuilt, but holds no positive observation (got '${populated:-<no response>}', want 1)"
    return 1
  fi
  return 0
}

# Block until EVERY gold observation table proves dbt refreshed it for THIS run.
#
# Container health only proves a process started, and a fixed sleep is a guess.
# Requiring the whole set rather than one canary is what stops a stand where a
# single generator family produced nothing from being reported ready — that
# failure otherwise surfaces much later, as tests failing on absent rows.
test_stand_wait_ready() {
  local run_started_at="$1"
  local elapsed=0 table reason
  local pending=() reasons=()

  echo "=== Readiness gate: waiting for ${#TEST_STAND_READY_TABLES[@]} gold observation tables rebuilt since ${run_started_at} ==="
  while [[ "$elapsed" -lt "$TEST_STAND_READY_TIMEOUT" ]]; do
    pending=(); reasons=()
    for table in "${TEST_STAND_READY_TABLES[@]}"; do
      # Declared above the assignment: `local reason=$(…)` would mask the exit
      # status of the command substitution behind `local`'s own.
      if ! reason="$(test_stand_table_ready "$table" "$run_started_at")"; then
        pending+=("$table")
        reasons+=("         insight.${table}: ${reason}")
      fi
    done

    if [[ ${#pending[@]} -eq 0 ]]; then
      echo "Readiness gate: all ${#TEST_STAND_READY_TABLES[@]} gold observation tables rebuilt and populated after ${elapsed}s."
      return 0
    fi

    sleep "$TEST_STAND_READY_INTERVAL"
    elapsed=$((elapsed + TEST_STAND_READY_INTERVAL))
  done

  # Every failing table, not just the first: one generator family being empty
  # and dbt never having run at all look identical when only one name is shown.
  echo "ERROR: readiness gate timed out after ${TEST_STAND_READY_TIMEOUT}s." >&2
  echo "       ${#pending[@]} of ${#TEST_STAND_READY_TABLES[@]} gold observation tables never became ready:" >&2
  # Guarded: `set -u` makes expanding an empty array an error on bash < 4.4, and
  # a zero-iteration loop (a timeout of 0) would leave this one empty.
  if [[ ${#reasons[@]} -gt 0 ]]; then
    printf '%s\n' "${reasons[@]}" >&2
  fi
  echo "       The stack is still up. Re-run the seed with:" >&2
  echo "         ./dev-compose.sh test-stand seed" >&2
  return 1
}

# The gateway's port INSIDE its own container (docker-compose.yml publishes it
# as "${GATEWAY_PORT:-8080}:8080"). A runner sharing the gateway's network
# namespace talks to this, not to the published host port.
TEST_STAND_GATEWAY_CONTAINER_PORT=8080
# docker-compose.yml pins `container_name: insight-gateway`, so the namespace to
# join is a fixed name rather than something to discover.
TEST_STAND_GATEWAY_CONTAINER=insight-gateway
# Where pytest-playwright writes traces, screenshots and video. `test-results`
# is its own default and the image's WORKDIR is /tests, so mounting the host
# directory at /tests/test-results makes the default land on the host with no
# --output flag to keep in step.
TEST_STAND_ARTIFACT_DIR="test-results"

# Run the suite inside the published ui-tests image against the running stand.
#
# The image is never built here: CI pulls it, a developer builds it once by
# hand (see deploy/compose/ui-tests.Dockerfile). This function only wires it to
# the stand, and the wiring is the part that is easy to get wrong.
#
# Network namespace, not the compose network. The session cookie is
# `__Host-`-prefixed, so the browser stores it only from a trustworthy origin,
# and over plain http `localhost` is the only host name that qualifies. Joining
# the gateway's namespace means one URL — http://localhost:8080 — serves the
# browser and the HTTP clients alike, with no Chromium flags (which do not lift
# the restriction anyway — measured, see tests/stand/ui/conftest.py).
#
# Arguments are passed to pytest verbatim and are IMAGE-SIDE paths: the suite
# lives at /tests/stand in the image, so select with /tests/stand/ui, not
# tests/stand/ui.
test_stand_test_in_image() {
  local image="$1" gw_port="$2"
  shift 2

  # The realm registers http://localhost:${GATEWAY_PORT}/auth/callback while an
  # in-namespace browser reaches the gateway at its container port. When those
  # differ the OIDC redirect_uri does not match and the login fails several
  # steps later, as an opaque IdP error. Refuse up front instead.
  if [[ "$gw_port" != "$TEST_STAND_GATEWAY_CONTAINER_PORT" ]]; then
    echo "ERROR: --image needs GATEWAY_PORT=${TEST_STAND_GATEWAY_CONTAINER_PORT} (this stand: ${gw_port})." >&2
    echo "       A containerised runner reaches the gateway at its container port," >&2
    echo "       but the realm registered http://localhost:${gw_port}/auth/callback," >&2
    echo "       so the login would fail on a redirect_uri mismatch." >&2
    return 1
  fi

  local manifest="deploy/seed/manifest.json"
  [[ -f "$manifest" ]] || {
    echo "ERROR: $manifest not found — seed the stand first: ./dev-compose.sh test-stand seed" >&2
    return 1; }

  docker image inspect "$image" >/dev/null 2>&1 || {
    echo "ERROR: image '$image' is not present locally. Pull it first:" >&2
    echo "         docker pull $image" >&2
    echo "       This verb never builds it — the image is a published artefact." >&2
    return 1; }

  # Created here rather than by the container, so it belongs to the invoking
  # user instead of root and the artefacts stay readable afterwards.
  mkdir -p "$TEST_STAND_ARTIFACT_DIR"

  local run_args=(
    --rm
    # As the INVOKING user, not the image's declared one. The image drops root
    # (ui-tests.Dockerfile), but a bind-mounted artifact directory takes its
    # ownership from the host, so a container uid that does not match the host's
    # cannot write into it — and the traces a failed journey uploads are the
    # whole reason that mount exists.
    --user "$(id -u):$(id -g)"
    --network "container:${TEST_STAND_GATEWAY_CONTAINER}"
    -e "INSIGHT_STAND_BASE_URL=http://localhost:${TEST_STAND_GATEWAY_CONTAINER_PORT}"
    -v "$PWD/${manifest}:/deploy/seed/manifest.json:ro"
    -v "$PWD/${TEST_STAND_ARTIFACT_DIR}:/tests/${TEST_STAND_ARTIFACT_DIR}"
    # Named, not inferred. The suite otherwise resolves this by walking up from
    # its own file to the directory holding `tests/` — which is the repo root in
    # a checkout and `/` in this image, where the suite lives at /tests with
    # nothing above it. That wrote the ledger to /.artifacts, outside the mount,
    # and only worked at all because the image used to run as root.
    -e "INSIGHT_STAND_ARTIFACT_DIR=/tests/${TEST_STAND_ARTIFACT_DIR}"
  )

  # The persona password comes from the generated realm export when it is
  # readable, and from the environment otherwise. Mounting the realm keeps a
  # keycloak stand working with no secret to distribute; the env var stays the
  # path for a stand whose realm this checkout cannot see.
  local realm="deploy/compose/keycloak/realm-insight.generated.json"
  [[ -f "$realm" ]] && run_args+=(-v "$PWD/${realm}:/${realm}:ro")
  [[ -n "${INSIGHT_STAND_PERSONA_PASSWORD:-}" ]] && run_args+=(-e INSIGHT_STAND_PERSONA_PASSWORD)

  # Service-principal tests need the `testclient` private key to sign their
  # assertion with, and two IN-NETWORK addresses: this runner shares the
  # gateway's network namespace, so `localhost` is the gateway's and the
  # published AUTHENTICATOR_TOKEN_PORT / IDENTITY_RESOLUTION_PORT are not
  # reachable from in here. Both are service listeners the gateway does not
  # front — the token exchange, and identity's `/internal/*`, which a service
  # principal reaches directly because the edge refuses a bearer-only caller.
  # Without the key the suite skips those tests with a reason rather than
  # failing, so the mount is conditional on the key existing.
  local service_key="deploy/compose/authenticator-dev-keys/testclient.key.pem"
  if [[ -f "$service_key" ]]; then
    run_args+=(-v "$PWD/${service_key}:/${service_key}:ro")
    run_args+=(-e "INSIGHT_STAND_SERVICE_KEY=/${service_key}")
    run_args+=(-e "INSIGHT_STAND_TOKEN_URL=http://authenticator:8093")
    run_args+=(-e "INSIGHT_STAND_IDENTITY_URL=http://identity-resolution:8082")
  fi

  echo "=== running the suite in ${image} (namespace: ${TEST_STAND_GATEWAY_CONTAINER}) ==="
  docker run "${run_args[@]}" "$image" "$@"
}

cmd_test_stand() {
  local verb="${1:-help}"
  [[ $# -gt 0 ]] && shift

  case "$verb" in
    up)
      local auth_mode="keycloak" image build_backend=false
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --auth=*) auth_mode="${1#*=}"; shift ;;
          --auth)   auth_mode="$2"; shift 2 ;;
          --build-backend) build_backend=true; shift ;;
          -h|--help) cmd_test_stand_help; return 0 ;;
          *) echo "ERROR: unknown test-stand up option: $1" >&2; return 2 ;;
        esac
      done

      image="$(test_stand_frontend_image)" || return 1
      test_stand_write_env "$image" "$auth_mode" || return 1

      # Pinning writes the four *_IMAGE vars into the env file, which is what
      # makes cmd_up put those services in its ghcr list — so this has to happen
      # before cmd_up reads it.
      if [[ "$build_backend" != true ]]; then
        test_stand_backend_matches_charts || return 1
        test_stand_pull_backends || return 1
      else
        echo "=== --build-backend: compiling the backend from this tree ==="
      fi

      local up_args=(--env-file "$TEST_STAND_ENV_FILE")
      [[ "$auth_mode" == keycloak ]] && up_args+=(--authenticator-redirect "$(test_stand_origin)/auth/callback")
      cmd_up "${up_args[@]}" || return 1

      # cmd_up resolved and exported the issuer for this run; persist what it
      # chose rather than re-deriving it, so the seed container and the test
      # suite read exactly the value the stack is running with.
      update_env_var "$TEST_STAND_ENV_FILE" AUTH_MODE "${AUTH_MODE:-$auth_mode}"
      update_env_var "$TEST_STAND_ENV_FILE" AUTHENTICATOR_OIDC_ISSUER "${AUTHENTICATOR_OIDC_ISSUER:-}"
      echo "=== persisted AUTH_MODE=${AUTH_MODE:-$auth_mode} AUTHENTICATOR_OIDC_ISSUER=${AUTHENTICATOR_OIDC_ISSUER:-<empty>} ==="

      # Scope the gate to this run BEFORE seeding.
      local run_started_at
      run_started_at="$(date -u '+%Y-%m-%d %H:%M:%S')"

      # cmd_up's first-run auto-seed may already have run, but it ran before
      # the issuer was persisted — so its manifest would record the wrong IdP.
      # Re-seed explicitly now that the env file is complete.
      cmd_seed --env-file "$TEST_STAND_ENV_FILE" all || return 1

      test_stand_wait_ready "$run_started_at" || return 1

      echo "=== test-stand is ready ==="
      ;;

    seed)
      [[ -f "$TEST_STAND_ENV_FILE" ]] || {
        echo "ERROR: $TEST_STAND_ENV_FILE not found — run: ./dev-compose.sh test-stand up" >&2; return 1; }
      cmd_seed --env-file "$TEST_STAND_ENV_FILE" "${@:-all}"
      ;;

    test)
      # Runs against an already-up stand: never brings it up, seeds it, or
      # tears it down, so a failing suite leaves the stand intact to inspect.
      #
      # Two runners, one verb. On the host (default) the suite runs from
      # tests/ with uv. With --image it runs inside an already-pulled ui-tests
      # image instead, which is what CI uses: the browser, its version and the
      # locked dependency set then come from a published artefact rather than
      # from whatever the runner happens to have installed.
      local image=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --image=*) image="${1#*=}"; shift ;;
          --image)
            [[ $# -ge 2 ]] || { echo "ERROR: --image requires a value." >&2; return 2; }
            image="$2"; shift 2 ;;
          # Only a LEADING --image is ours; everything from here on is pytest's.
          *) break ;;
        esac
      done

      # Read the port from the stand's own env file rather than the ambient
      # shell, so a stand on a non-default GATEWAY_PORT is probed where it
      # actually listens.
      local gw_port
      gw_port="$(grep -E '^[[:space:]]*GATEWAY_PORT=' "$TEST_STAND_ENV_FILE" 2>/dev/null | tail -1 | cut -d= -f2)"
      gw_port="${gw_port:-${GATEWAY_PORT:-8080}}"
      curl -sf -o /dev/null --max-time 5 "http://localhost:${gw_port}/" 2>/dev/null || {
        echo "ERROR: gateway is not answering on http://localhost:${gw_port}/." >&2
        echo "       Bring the stand up first: ./dev-compose.sh test-stand up" >&2
        return 1; }

      if [[ -n "$image" ]]; then
        test_stand_test_in_image "$image" "$gw_port" "$@"
        return $?
      fi

      [[ -d tests/stand ]] || {
        echo "ERROR: tests/stand does not exist yet (it is created in a later phase)." >&2
        return 1; }
      [[ -f tests/pyproject.toml ]] || {
        echo "ERROR: tests/pyproject.toml not found — the suite has no dependency set." >&2
        return 1; }
      command -v uv >/dev/null 2>&1 || {
        echo "ERROR: uv not found on PATH. The suite's dependencies (pytest, httpx," >&2
        echo "       playwright) are locked in tests/uv.lock and installed with uv:" >&2
        echo "         brew install uv   # or: curl -LsSf https://astral.sh/uv/install.sh | sh" >&2
        return 1; }
      # --frozen: run exactly the locked dependency set, never re-resolve
      # silently, so the host runner and the ui-tests image stay identical.
      uv run --project tests --frozen pytest tests/stand "$@"
      ;;

    down)
      # Always --volumes: the next `up` must start from empty databases, so a
      # run's data comes only from its own seed.
      cmd_down --volumes --env-file "$TEST_STAND_ENV_FILE"
      ;;

    help|-h|--help) cmd_test_stand_help ;;
    *) echo "ERROR: unknown test-stand verb: $verb" >&2; cmd_test_stand_help; return 2 ;;
  esac
}

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

  test-stand <up|seed|test|down>
          The stack in TEST configuration, driven from its own
          .env.compose.test-stand so your .env.compose is untouched:
          pinned ghcr frontend, real Keycloak login, and a readiness
          gate that waits for dbt-built gold data rather than for
          containers to report healthy. `test-stand test` runs
          tests/stand against the running stand.

  help    Print this message.

Each subcommand has its own --help.
EOF
}

main() {
  local sub="${1:-help}"
  [[ $# -gt 0 ]] && shift
  local args=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --instance=*)
        COMPOSE_INSTANCE="${1#*=}"
        [[ -n "$COMPOSE_INSTANCE" ]] || { echo "ERROR: --instance requires a value." >&2; return 2; }
        shift ;;
      --instance)
        [[ $# -ge 2 ]] || { echo "ERROR: --instance requires a value." >&2; return 2; }
        COMPOSE_INSTANCE="$2"
        [[ -n "$COMPOSE_INSTANCE" ]] || { echo "ERROR: --instance requires a value." >&2; return 2; }
        shift 2 ;;
      *)
        args+=("$1")
        shift ;;
    esac
  done
  compose_project_name "$COMPOSE_INSTANCE" >/dev/null || return $?
  case "$sub" in
    up)    cmd_up    ${args[@]+"${args[@]}"} ;;
    down)  cmd_down  ${args[@]+"${args[@]}"} ;;
    build) cmd_build ${args[@]+"${args[@]}"} ;;
    seed)  cmd_seed  ${args[@]+"${args[@]}"} ;;
    urls)  cmd_urls  ${args[@]+"${args[@]}"} ;;
    prune) cmd_prune ${args[@]+"${args[@]}"} ;;
    test-stand) cmd_test_stand ${args[@]+"${args[@]}"} ;;
    help|-h|--help) usage ;;
    *) echo "ERROR: unknown subcommand: $sub" >&2; usage; return 2 ;;
  esac
}

main "$@"

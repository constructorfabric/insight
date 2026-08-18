#!/usr/bin/env bash
# Seal every credential in secrets.plain.env into the committed SealedSecret
# manifests under sealed-secrets/.
#
#   ./seal-secrets.sh [--file secrets.plain.env] [--dry-run]
#
# Each value is sealed into EVERY Secret that carries it, so the copies cannot
# drift. A variable left empty is skipped, and the manifest keeps whatever it
# already held — that is how you rotate one credential without touching the
# rest.
#
# Requires kubeseal and a kubeconfig pointing at this stand: kubeseal fetches
# the controller's public certificate, so the sealed output is valid for this
# cluster only.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

ENV_FILE="secrets.plain.env"
DRY_RUN=false
while [ $# -gt 0 ]; do
  case "$1" in
    --file) ENV_FILE="$2"; shift 2 ;;
    --dry-run) DRY_RUN=true; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -f "$ENV_FILE" ] || {
  echo "ERROR: $ENV_FILE not found. Copy secrets.plain.env.template and fill it in." >&2
  exit 1
}
command -v kubeseal >/dev/null || { echo "ERROR: kubeseal not on PATH" >&2; exit 1; }

CONTROLLER_NAME="${CONTROLLER_NAME:-sealed-secrets-controller}"
CONTROLLER_NS="${CONTROLLER_NS:-kube-system}"

set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a

# seal <manifest> <secret-name> <namespace> <key>=<var> ...
#
# Builds a plain Secret from the named variables, seals it, and merges the
# result into the existing manifest. --merge-into is what preserves each
# manifest's template block (the connector annotations reconcile reads) and
# every key this call does not name.
seal() {
  local manifest="$1" name="$2" ns="$3"; shift 3
  local -a literals=() named=()

  local pair key var
  for pair in "$@"; do
    key="${pair%%=*}"; var="${pair#*=}"
    if [ -n "${!var:-}" ]; then
      literals+=(--from-literal="$key=${!var}")
      named+=("$key")
    fi
  done

  if [ ${#literals[@]} -eq 0 ]; then
    printf '  %-46s skipped (no values set)\n' "$name"
    return 0
  fi
  if [ ! -f "$manifest" ]; then
    printf '  %-46s SKIPPED — %s does not exist\n' "$name" "$manifest" >&2
    return 0
  fi

  if [ "$DRY_RUN" = true ]; then
    printf '  %-46s would seal: %s\n' "$name" "${named[*]}"
    return 0
  fi

  kubectl create secret generic "$name" --namespace "$ns" --dry-run=client \
    "${literals[@]}" -o yaml \
    | kubeseal --controller-name "$CONTROLLER_NAME" \
               --controller-namespace "$CONTROLLER_NS" \
               --format yaml --merge-into "$manifest"

  printf '  %-46s sealed: %s\n' "$name" "${named[*]}"
}

echo "Sealing from $ENV_FILE against $CONTROLLER_NS/$CONTROLLER_NAME"

# Datastore passwords: once in the datastore's own namespace, once in the
# composed insight-db-creds the services read.
seal sealed-secrets/clickhouse/clickhouse-insight-credentials-sealedsecret.yaml \
     clickhouse-insight-credentials clickhouse password=CLICKHOUSE_INSIGHT_PASSWORD
seal sealed-secrets/clickhouse/clickhouse-default-credentials-sealedsecret.yaml \
     clickhouse-default-credentials clickhouse password=CLICKHOUSE_DEFAULT_PASSWORD
seal sealed-secrets/mariadb/mariadb-insight-credentials-sealedsecret.yaml \
     mariadb-insight-credentials mariadb password=MARIADB_INSIGHT_PASSWORD
seal sealed-secrets/mariadb/mariadb-root-sealedsecret.yaml \
     mariadb-root mariadb password=MARIADB_ROOT_PASSWORD
seal sealed-secrets/redis/redis-auth-sealedsecret.yaml \
     redis-auth redis password=REDIS_PASSWORD

seal sealed-secrets/insight/insight-db-creds-sealedsecret.yaml \
     insight-db-creds insight \
     clickhouse-password=CLICKHOUSE_INSIGHT_PASSWORD \
     mariadb-password=MARIADB_INSIGHT_PASSWORD \
     mariadb-root-password=MARIADB_ROOT_PASSWORD \
     redis-password=REDIS_PASSWORD

# Keycloak admin, the brokered GitHub app, and the authenticator's client
# secret — each sealed into both the Secret that owns it and the config Job's
# environment, which is what writes them into the realm.
seal sealed-secrets/insight/insight-keycloak-admin-sealedsecret.yaml \
     insight-keycloak-admin insight username=KEYCLOAK_USER password=KEYCLOAK_PASSWORD
seal sealed-secrets/insight/github-oauth-sealedsecret.yaml \
     github-oauth insight client-id=GITHUB_CLIENT_ID client-secret=GITHUB_CLIENT_SECRET
seal sealed-secrets/insight/insight-oidc-sealedsecret.yaml \
     insight-oidc insight client-secret=AUTHENTICATOR_CLIENT_SECRET

seal sealed-secrets/insight/insight-keycloak-config-sealedsecret.yaml \
     insight-keycloak-config insight \
     KEYCLOAK_USER=KEYCLOAK_USER \
     KEYCLOAK_PASSWORD=KEYCLOAK_PASSWORD \
     GITHUB_CLIENT_ID=GITHUB_CLIENT_ID \
     GITHUB_CLIENT_SECRET=GITHUB_CLIENT_SECRET \
     INSIGHT_AUTHENTICATOR_CLIENT_SECRET=AUTHENTICATOR_CLIENT_SECRET

# Connector credentials.
seal sealed-secrets/insight/insight-github-directory-main-sealedsecret.yaml \
     insight-github-directory-main insight \
     github_token=GITHUB_DIRECTORY_TOKEN \
     github_organizations=GITHUB_DIRECTORY_ORGANIZATIONS

seal sealed-secrets/insight/insight-github-main-sealedsecret.yaml \
     insight-github-main insight \
     github_token=GITHUB_TOKEN \
     github_organizations=GITHUB_ORGANIZATIONS \
     github_start_date=GITHUB_START_DATE \
     git_proxy_url=GIT_PROXY_URL \
     git_proxy_token=GIT_PROXY_TOKEN

cat <<'NOTE'

Done. Next:
  git diff --stat                     # only encryptedData lines should move
  make secrets                        # apply the sealed manifests
  make deploy CONFIRM=yes-deploy-prod # re-run the keycloak-config hook

kubeseal rewrites each manifest through a YAML parser, so any COMMENT in a
sealed-secrets file is dropped. Check `git diff` for lost comment lines and
restore them before committing.
NOTE

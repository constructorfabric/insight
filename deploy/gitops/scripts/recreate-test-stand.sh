#!/usr/bin/env bash
# Rebuild a disposable stand: uninstall the release, drop the application
# DATABASES, reinstall. Every precondition resolves before the first
# destructive command. Safe to re-run; plans unless --apply. See INFRA.md.
#
# Usage:
#   recreate-test-stand.sh --expect-cluster <name> [--apply] [options]
#
#   --kubeconfig PATH      kubeconfig to act through (default: ambient)
#   --context NAME         context within it (default: its current-context)
#   --expect-cluster NAME  REQUIRED. The cluster the context must resolve to.
#   --env NAME             gitops environment (default: test-stand)
#   --confirm TOKEN        required with --apply: `wipe-<env>`
#   --apply                actually do it
#   --keep-databases       uninstall+redeploy without dropping any database
#   --no-deploy            wipe only; do not reinstall. The stand is left EMPTY.
#   --timeout DURATION     helm timeout for the reinstall (default: 10m)
#   --version X.Y.Z        chart version (default: latest published — INFRA.md, "fork-downgrade wedge")
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GITOPS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

ENV_NAME="test-stand"
KUBECONFIG_IN=""
CONTEXT=""
EXPECT_CLUSTER=""
CONFIRM=""
APPLY=0
KEEP_DATABASES=0
DO_DEPLOY=1
TIMEOUT="10m"
VERSION=""
UPSTREAM_REPO="constructorfabric/insight"
DEFAULT_BRANCH="main"

usage() { sed -n '/^# Usage:/,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//; $d' >&2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --kubeconfig)     KUBECONFIG_IN="${2:?--kubeconfig needs a path}"; shift 2 ;;
    --context)        CONTEXT="${2:?--context needs a name}"; shift 2 ;;
    --expect-cluster) EXPECT_CLUSTER="${2:?--expect-cluster needs a name}"; shift 2 ;;
    --env)            ENV_NAME="${2:?--env needs a name}"; shift 2 ;;
    --confirm)        CONFIRM="${2:?--confirm needs a token}"; shift 2 ;;
    --timeout)        TIMEOUT="${2:?--timeout needs a duration}"; shift 2 ;;
    --version)        VERSION="${2:?--version needs a semver}"; shift 2 ;;
    --apply)          APPLY=1; shift ;;
    --keep-databases) KEEP_DATABASES=1; shift ;;
    --no-deploy)      DO_DEPLOY=0; shift ;;
    -h|--help)        usage; exit 0 ;;
    *)                die "unknown argument: $1" ;;
  esac
done

INVENTORY="$GITOPS_DIR/environments/$ENV_NAME/inventory.yaml"
VALUES="$GITOPS_DIR/environments/$ENV_NAME/values.yaml"
[ -f "$INVENTORY" ] || die "no inventory at $INVENTORY"
[ -f "$VALUES" ]    || die "no values at $VALUES"

NAMESPACE="$(yq -r '.namespaces.services // "insight"' "$INVENTORY")"
RELEASE="$(yq -r '.release // "insight"' "$INVENTORY")"
IDENTITY_DB="$(yq -r '.identityResolution.databaseName // "identity"' "$VALUES")"
KEYCLOAK_DB="$(yq -r '.keycloak.database.name // "keycloak"' "$VALUES")"
ANALYTICS_DB="$(yq -r '.mariadb.database // "insight"' "$VALUES")"
DB_USER="$(yq -r '.mariadb.username // "insight"' "$VALUES")"
MARIADB_HOST="$(yq -r '.mariadb.host' "$VALUES")"
MARIADB_PORT="$(yq -r '.mariadb.port // 3306' "$VALUES")"
CH_HOST="$(yq -r '.clickhouse.host' "$VALUES")"
CH_PORT="$(yq -r '.clickhouse.port // 8123' "$VALUES")"
CH_USER="$(yq -r '.clickhouse.username // "insight"' "$VALUES")"

MARIADB_IMAGE="docker.io/bitnamilegacy/mariadb:11.4.4-debian-12-r0"  # mirrors templates/mariadb-init-svcdbs-job.yaml
CURL_IMAGE="curlimages/curl:8.10.1"  # mirrors templates/clickhouse-init-svcdbs-job.yaml

# Not helm-managed; must survive the wipe. LIMIT: hand-maintained list — a
# newly-required secret not added here is wiped through silently.
SURVIVORS_SECRET=(
  insight-db-creds
  insight-oidc
  insight-keycloak-admin
  insight-keycloak-config
  insight-authenticator-signing-keys
)
SURVIVORS_CM=("${RELEASE}-keycloak-config-realms")

TMP_PRECHECK="$(mktemp)"
trap 'rm -f "$TMP_PRECHECK"' EXIT INT TERM

KUBECTL=(kubectl)
[ -n "$KUBECONFIG_IN" ] && KUBECTL+=(--kubeconfig "$KUBECONFIG_IN")
[ -n "$CONTEXT" ] && KUBECTL+=(--context "$CONTEXT")
HELM=(helm)
[ -n "$KUBECONFIG_IN" ] && HELM+=(--kubeconfig "$KUBECONFIG_IN")
[ -n "$CONTEXT" ] && HELM+=(--kube-context "$CONTEXT")

# Guarded on the CLUSTER a context resolves to, never the context NAME.
hdr "target"

if [ -z "$CONTEXT" ]; then
  CONTEXT="$("${KUBECTL[@]}" config current-context 2>/dev/null || true)"
  [ -n "$CONTEXT" ] || die "no current-context and --context was not given"
  KUBECTL+=(--context "$CONTEXT")
  HELM+=(--kube-context "$CONTEXT")
fi

[ -n "$EXPECT_CLUSTER" ] || die "--expect-cluster is required. There is no default: this script destroys data, and the one thing it must never do is destroy somebody else's."

cluster_guard "$CONTEXT" "$EXPECT_CLUSTER"   # from lib.sh; sets ACTUAL_CLUSTER
note "context   : $CONTEXT"
note "cluster   : $ACTUAL_CLUSTER"
note "namespace : $NAMESPACE"
note "release   : $RELEASE"
note "env       : $ENV_NAME"

namespace_assert "$NAMESPACE"   # from lib.sh

EXPECT_CONFIRM="wipe-$ENV_NAME"
if [ "$APPLY" = "1" ] && [ "$CONFIRM" != "$EXPECT_CONFIRM" ]; then
  die "--apply requires --confirm $EXPECT_CONFIRM (a literal token, so it cannot be produced by a variable that happens to be wrong)"
fi

# Checked BEFORE the uninstall, because after it they are the only way back.
hdr "survivors — what must outlive the wipe"

missing=0
for s in "${SURVIVORS_SECRET[@]}"; do
  if "${KUBECTL[@]}" -n "$NAMESPACE" get secret "$s" -o name >/dev/null 2>&1; then
    note "${C_GRN}ok${C_RST}    secret/$s"
  else
    note "${C_RED}MISSING${C_RST} secret/$s"; missing=1
  fi
done
for c in "${SURVIVORS_CM[@]}"; do
  if "${KUBECTL[@]}" -n "$NAMESPACE" get configmap "$c" -o name >/dev/null 2>&1; then
    note "${C_GRN}ok${C_RST}    configmap/$c"
  else
    note "${C_RED}MISSING${C_RST} configmap/$c"; missing=1
  fi
done

# Presence is not enough: an empty client-secret leaves every pod Ready and
# every login failing at the code exchange.
if "${KUBECTL[@]}" -n "$NAMESPACE" get secret insight-oidc -o name >/dev/null 2>&1; then
  oidc_secret_nonempty "$NAMESPACE" insight-oidc client-secret || missing=1   # from lib.sh
fi

if [ "$missing" = "1" ]; then
  die "refusing to wipe: something the rebuilt stand needs is absent. Restore it first — these objects are created by the bring-up outside this repository, not by the chart."
fi

hdr "regenerated by the chart (destroyed on purpose)"
"${KUBECTL[@]}" -n "$NAMESPACE" get secret,configmap \
  -l app.kubernetes.io/managed-by=Helm -o name 2>/dev/null | sed 's/^/  /' >&2 || true

# DROP proved by creating and dropping a scratch database, not by parsing
# grant text, which a role-based grant satisfies without matching a pattern.
hdr "datastore preflight (verifies DROP capability directly)"

PREFLIGHT_JOB="recreate-preflight-$(date -u +%Y%m%d%H%M%S)"
preflight_manifest() {
  cat <<YAML
apiVersion: batch/v1
kind: Job
metadata:
  name: ${PREFLIGHT_JOB}
  namespace: ${NAMESPACE}
  labels: {app.kubernetes.io/managed-by: recreate-test-stand.sh}
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels: {app.kubernetes.io/managed-by: recreate-test-stand.sh}
    spec:
      restartPolicy: Never
      # Two containers, each with the client image that actually carries its
      # client (mariadb has no curl); read-only here, so the init-then-main
      # ordering is for legibility (it is load-bearing in the wipe below).
      initContainers:
        - name: mariadb
          image: ${MARIADB_IMAGE}
          env:
            # EXPLICIT secretKeyRef names, never envFrom: the keys in
            # insight-db-creds are hyphenated, not valid shell identifiers.
            - {name: MYSQL_PWD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: mariadb-root-password}}}
          command:
            - bash
            - -ec
            - |
              echo "MariaDB ${MARIADB_HOST}:${MARIADB_PORT} as root"
              mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot -e 'SELECT 1' >/dev/null
              for db in '${IDENTITY_DB}' '${KEYCLOAK_DB}' '${ANALYTICS_DB}'; do
                n=\$(mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot -N -B \
                      -e "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='\$db'" 2>/dev/null)
                echo "  \$db: present=\$n"
              done
              mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot \
                -e "CREATE DATABASE IF NOT EXISTS \\\`__cf_probe__\\\`; DROP DATABASE \\\`__cf_probe__\\\`" \
                || { echo "root cannot CREATE+DROP a scratch database — refusing"; exit 1; }
              echo "  root can DROP: yes (verified: created+dropped __cf_probe__)"
      containers:
        - name: clickhouse
          image: ${CURL_IMAGE}
          env:
            - {name: CH_PASSWORD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: clickhouse-password}}}
          command:
            - sh
            - -ec
            - |
              echo "ClickHouse ${CH_HOST}:${CH_PORT} as ${CH_USER}"
              code=\$(curl -s -o /tmp/ch -w '%{http_code}' --max-time 20 \
                -H "X-ClickHouse-User: ${CH_USER}" \
                -H "X-ClickHouse-Key: \$CH_PASSWORD" \
                --data-binary 'SELECT count() FROM system.databases' \
                'http://${CH_HOST}:${CH_PORT}/')
              [ "\$code" = "200" ] || { echo "ClickHouse answered \$code"; cat /tmp/ch; exit 1; }
              echo "  databases visible: \$(cat /tmp/ch)"
              # Authorisation, not just reachability — proved directly rather
              # than by parsing SHOW GRANTS text, which a role-based grant
              # can satisfy without matching a fixed pattern.
              code=\$(curl -s -o /tmp/g -w '%{http_code}' --max-time 20 \
                -H "X-ClickHouse-User: ${CH_USER}" \
                -H "X-ClickHouse-Key: \$CH_PASSWORD" \
                --data-binary 'CREATE DATABASE IF NOT EXISTS __cf_probe__' \
                'http://${CH_HOST}:${CH_PORT}/')
              [ "\$code" = "200" ] || { echo "CREATE DATABASE probe answered \$code"; cat /tmp/g; exit 1; }
              code=\$(curl -s -o /tmp/g -w '%{http_code}' --max-time 20 \
                -H "X-ClickHouse-User: ${CH_USER}" \
                -H "X-ClickHouse-Key: \$CH_PASSWORD" \
                --data-binary 'DROP DATABASE __cf_probe__' \
                'http://${CH_HOST}:${CH_PORT}/')
              [ "\$code" = "200" ] || { echo "DROP DATABASE probe answered \$code"; cat /tmp/g; exit 1; }
              echo "  ${CH_USER} can DROP: yes (verified: created+dropped __cf_probe__)"
              echo "PREFLIGHT OK"
YAML
}

run_job() {
  local name="$1" manifest_fn="$2" timeout="$3"
  "$manifest_fn" | "${KUBECTL[@]}" apply -f - >/dev/null
  if ! "${KUBECTL[@]}" -n "$NAMESPACE" wait --for=condition=complete --timeout="$timeout" "job/$name" >/dev/null 2>&1; then
    "${KUBECTL[@]}" -n "$NAMESPACE" logs "job/$name" --all-containers=true 2>&1 | sed 's/^/    /' >&2 || true
    "${KUBECTL[@]}" -n "$NAMESPACE" delete "job/$name" --ignore-not-found >/dev/null 2>&1 || true
    return 1
  fi
  "${KUBECTL[@]}" -n "$NAMESPACE" logs "job/$name" --all-containers=true 2>&1 | sed 's/^/    /' >&2 || true
  "${KUBECTL[@]}" -n "$NAMESPACE" delete "job/$name" --ignore-not-found >/dev/null 2>&1 || true
  return 0
}

if [ "$APPLY" = "1" ]; then
  run_job "$PREFLIGHT_JOB" preflight_manifest 180s \
    || die "datastore preflight failed — nothing has been destroyed. Fix the connection or the grant and re-run."
  note "${C_GRN}preflight passed${C_RST}"
else
  note "would run a Job asserting MariaDB and ClickHouse are reachable, then verify"
  note "DROP capability by creating and dropping a scratch __cf_probe__ database in each"
fi

# `make deploy`'s own guards, against the same target and arguments: a refusal
# now costs nothing, the same refusal after the wipe costs the stand.
if [ "$DO_DEPLOY" = "1" ]; then
  hdr "deploy preconditions (before anything is destroyed)"
  if [ -z "$VERSION" ]; then
    VERSION="$(gh api "repos/${UPSTREAM_REPO}/contents/deploy/gitops/.insight-version?ref=${DEFAULT_BRANCH}" \
                 --jq '.content' 2>/dev/null | base64 --decode | tr -d '[:space:]' || true)"
    [ -n "$VERSION" ] || die "could not resolve the published version from ${UPSTREAM_REPO}@${DEFAULT_BRANCH}. Pass --version X.Y.Z explicitly — deliberately NOT falling back to $GITOPS_DIR/.insight-version, which is as old as this branch."
  fi
  note "chart to install: insight-$VERSION"
  if ! make -C "$GITOPS_DIR" --no-print-directory \
        sync-clean vpn-up kube-ctx values-present chart-present \
        ENV="$ENV_NAME" KUBE_CTX="$CONTEXT" ${KUBECONFIG_IN:+KUBECONFIG="$KUBECONFIG_IN"} \
        INSIGHT_VERSION="$VERSION" >/dev/null 2>"$TMP_PRECHECK"; then
    sed 's/^/    /' <"$TMP_PRECHECK" >&2
    die "the deploy would be refused AFTER the wipe — refusing now instead. Nothing has been destroyed."
  fi
  note "${C_GRN}the deploy's own guards pass${C_RST} — a wipe now can be followed by an install"

  # The Makefile is the one place the deploy CONFIRM token format is defined;
  # consumed here, never re-derived, so the two cannot drift.
  DEPLOY_CONFIRM="$(make -C "$GITOPS_DIR" --no-print-directory -s print-confirm-token ENV="$ENV_NAME")"
  [ -n "$DEPLOY_CONFIRM" ] || die "could not resolve the deploy CONFIRM token via 'make print-confirm-token' in $GITOPS_DIR"
fi

hdr "plan"
note "1. helm uninstall $RELEASE -n $NAMESPACE   (the release only)"
if [ "$KEEP_DATABASES" = "1" ]; then
  note "2. ${C_YEL}SKIPPED${C_RST} database wipe (--keep-databases)"
  note "   NOTE: identity.persons is append-only. Stale observations from earlier"
  note "   seeds survive this, and they are the reason this script exists."
else
  note "2. drop MariaDB   ${IDENTITY_DB}, ${KEYCLOAK_DB}, ${ANALYTICS_DB}"
  note "   (${IDENTITY_DB} and ${KEYCLOAK_DB} are recreated + granted by the chart's"
  note "    pre-install hook; ${ANALYTICS_DB} is recreated here, because that hook"
  note "    does not create the analytics database and the operator's Database CR"
  note "    is cleanupPolicy: Skip)"
  note "   drop ClickHouse every database except system/INFORMATION_SCHEMA/default,"
  note "   whatever it is currently named (the seed's create-bronze-placeholders.sh"
  note "   and the chart's migrate hook rebuild them)"
  note "   RISK: on a shared ClickHouse host this drops every non-system database"
  note "   ${CH_USER} can see, not just this stand's"
fi
if [ "$DO_DEPLOY" = "1" ]; then
  note "3. make deploy ENV=$ENV_NAME  (the same target CI runs)"
  note "4. verify the release is deployed and the public edge answers"
else
  note "3. ${C_YEL}SKIPPED${C_RST} deploy (--no-deploy) — the stand is left EMPTY"
fi

if [ "$APPLY" != "1" ]; then
  hdr "dry run"
  note "Nothing was destroyed."
  note "Re-run with ${C_CYA}--apply --confirm $EXPECT_CONFIRM${C_RST} to execute."
  exit 0
fi

hdr "uninstall"
if "${HELM[@]}" status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
  "${HELM[@]}" uninstall "$RELEASE" -n "$NAMESPACE" --wait --timeout 5m 2>&1 | sed 's/^/  /' >&2
else
  note "no release named $RELEASE — nothing to uninstall"
fi

# A FAILED hook Job survives both its own hook-delete-policy and `helm uninstall`.
"${KUBECTL[@]}" -n "$NAMESPACE" delete jobs -l app.kubernetes.io/managed-by=Helm --ignore-not-found >/dev/null 2>&1 || true

if [ "$KEEP_DATABASES" != "1" ]; then
  hdr "database wipe"
  WIPE_JOB="recreate-wipe-$(date -u +%Y%m%d%H%M%S)"
  wipe_manifest() {
    cat <<YAML
apiVersion: batch/v1
kind: Job
metadata:
  name: ${WIPE_JOB}
  namespace: ${NAMESPACE}
  labels: {app.kubernetes.io/managed-by: recreate-test-stand.sh}
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  template:
    metadata:
      labels: {app.kubernetes.io/managed-by: recreate-test-stand.sh}
    spec:
      restartPolicy: Never
      # Ordering IS load-bearing here (unlike the preflight above): the init
      # must complete before the main container starts, so a MariaDB failure
      # stops the run before ClickHouse is touched.
      initContainers:
        - name: mariadb
          image: ${MARIADB_IMAGE}
          env:
            - {name: MYSQL_PWD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: mariadb-root-password}}}
          command:
            - bash
            - -ec
            - |
              M() { mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot "\$@"; }
              echo "MariaDB: dropping ${IDENTITY_DB}, ${KEYCLOAK_DB}, ${ANALYTICS_DB}"
              M -e "DROP DATABASE IF EXISTS \\\`${IDENTITY_DB}\\\`"
              M -e "DROP DATABASE IF EXISTS \\\`${KEYCLOAK_DB}\\\`"
              M -e "DROP DATABASE IF EXISTS \\\`${ANALYTICS_DB}\\\`"

              # Recreated HERE, unlike the other two: the chart's install hook
              # never creates the analytics database, and the operator's
              # Database CR is cleanupPolicy: Skip.
              echo "MariaDB: recreating ${ANALYTICS_DB} and granting ${DB_USER}"
              M -e "CREATE DATABASE \\\`${ANALYTICS_DB}\\\` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
              M -e "GRANT ALL PRIVILEGES ON \\\`${ANALYTICS_DB}\\\`.* TO \\\`${DB_USER}\\\`@\\\`%\\\`"
              M -e "FLUSH PRIVILEGES"
              echo "MARIADB WIPE OK"
      containers:
        - name: clickhouse
          image: ${CURL_IMAGE}
          env:
            - {name: CH_PASSWORD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: clickhouse-password}}}
          command:
            - sh
            - -ec
            - |
              CH() {
                curl -sf --max-time 120 \
                  -H "X-ClickHouse-User: ${CH_USER}" \
                  -H "X-ClickHouse-Key: \$CH_PASSWORD" \
                  --data-binary "\$1" 'http://${CH_HOST}:${CH_PORT}/'
              }
              echo "ClickHouse: dropping application databases"
              DBS=\$(CH "SELECT name FROM system.databases WHERE name NOT IN ('system','INFORMATION_SCHEMA','information_schema','default') FORMAT TSV")
              for db in \$DBS; do
                echo "  drop \$db"
                CH "DROP DATABASE IF EXISTS \\\`\$db\\\`" >/dev/null
              done
              echo "WIPE OK"
YAML
  }
  run_job "$WIPE_JOB" wipe_manifest 600s || die "database wipe failed — see the log above. The release is already uninstalled; fix the cause and re-run."
  note "${C_GRN}databases wiped${C_RST}"
fi

if [ "$DO_DEPLOY" = "1" ]; then
  hdr "deploy"
  # Resolved from the default branch through the API, never from this
  # checkout's .insight-version. See INFRA.md, "fork-downgrade wedge".
  note "installing insight-$VERSION (resolved in the preconditions above)"

  LOCAL_FILE="$(cat "$GITOPS_DIR/.insight-version" 2>/dev/null || true)"
  if [ -n "$LOCAL_FILE" ] && [ "$LOCAL_FILE" != "$VERSION" ]; then
    note "${C_YEL}note${C_RST}  this checkout's .insight-version says $LOCAL_FILE — ignored, see above"
  fi
  note "installing insight-$VERSION through the official target"
  # Context passed through, not rediscovered: the cluster guard above is
  # stronger than the Makefile's own context-NAME check.
  make -C "$GITOPS_DIR" --no-print-directory deploy \
    ENV="$ENV_NAME" CONFIRM="$DEPLOY_CONFIRM" \
    KUBE_CTX="$CONTEXT" ${KUBECONFIG_IN:+KUBECONFIG="$KUBECONFIG_IN"} \
    INSIGHT_VERSION="$VERSION" TIMEOUT="$TIMEOUT" 2>&1 | sed 's/^/  /' >&2

  hdr "verify"
  make -C "$GITOPS_DIR" --no-print-directory verify-release \
    ENV="$ENV_NAME" KUBE_CTX="$CONTEXT" ${KUBECONFIG_IN:+KUBECONFIG="$KUBECONFIG_IN"} \
    INSIGHT_VERSION="$VERSION" 2>&1 | sed 's/^/  /' >&2

  BASE="https://$(yq -r '.gateway.route.host' "$VALUES")"
  for probe in "/:200" "/auth/me:401"; do
    path="${probe%:*}"; want="${probe##*:}"
    code="$(curl -s -o /dev/null -w '%{http_code}' -m 20 "$BASE$path" || echo 000)"
    if [ "$code" = "$want" ]; then note "${C_GRN}ok${C_RST}    $code  $path"
    else note "${C_RED}FAIL${C_RST}  $code  $path (expected $want)"; fi
  done
fi

hdr "next"
note "The stand is rebuilt and EMPTY of application data. Seed it:"
note ""
note "  src/ingestion/tools/seed/seed-stand.sh -n $NAMESPACE --context $CONTEXT --days 365"
note ""
note "365 and not more: the analytics API refuses a period of 400 days or more,"
note "and the stand suites ask about the window the manifest reports."
note ""
note "CAVEAT — Airbyte connector state. This wipe drops the bronze_* databases an"
note "Airbyte destination writes into, but Airbyte's per-connection cursors live"
note "in Airbyte's own store and are NOT reset here. An incremental connection"
note "will resume from its cursor and never re-emit the history it has already"
note "recorded as synced. If this stand ever runs real connectors, reset them in"
note "Airbyte after a wipe, or the bronze layer will be silently short."

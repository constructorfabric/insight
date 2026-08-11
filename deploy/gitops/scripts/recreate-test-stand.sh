#!/usr/bin/env bash
# Rebuild a disposable stand from the state it is in to the state CI expects.
#
# WHY THIS EXISTS, AND WHY `helm uninstall` IS NOT IT
# ---------------------------------------------------
# The recovery model for this stand is wipe-and-recreate rather than rollback:
# rolling back reinstalls the PREVIOUS chart, which is a downgrade onto a stand
# whose entire job is to show what the newest one does, and it leaves live
# objects in the old chart's shape that the next upgrade cannot patch.
#
# But a helm uninstall is NOT a wipe, and believing it is cost a full afternoon
# once. The `insight` namespace holds six Deployments and nothing else — no
# StatefulSets, no PersistentVolumeClaims. MariaDB, ClickHouse, Redis and
# Redpanda are operator-managed in their OWN namespaces and are untouched by
# anything helm does to this release. So a "clean redeploy" leaves every row of
# application data exactly where it was.
#
# That is not academic. `identity.persons` is an APPEND-ONLY observation log. A
# single stale email observation, written by a seed run five hours earlier and
# carried across a full uninstall/reinstall, made five API tests fail in a way
# that read convincingly as a product defect — profiles-by-email 404, a session
# resolving to the wrong person, a visibility grant that would not apply. It was
# diagnosed as a product bug twice before anybody looked at the row. A stand you
# cannot fully recreate is a stand whose red runs you cannot trust.
#
# So this script drops the application DATABASES as well as the release.
#
# WHAT IT PRESERVES, AND WHY IT CHECKS FIRST
# ------------------------------------------
# Several Secrets in the namespace are NOT helm-managed and survive an uninstall
# by design — the database credentials, the OIDC client secret, the Keycloak
# bootstrap admin, the authenticator's signing keys — along with the realm
# ConfigMap. They are also the only way back: without them the reinstalled
# release cannot reach its datastores, cannot complete a login, and cannot
# re-import the realm. They are verified BEFORE anything is destroyed, and a
# missing one is a refusal rather than a warning.
#
# ORDER, AND WHY IT IS THIS ORDER
# -------------------------------
#   1. guards            — right cluster, right namespace, explicit confirmation
#   2. survivors         — the credentials that must outlive the wipe are present
#   3. datastore preflight — MariaDB and ClickHouse are reachable AND authorised
#   4. helm uninstall    — the release
#   5. database wipe     — one Job, both datastores, after (3) proved both work
#   6. deploy            — `make deploy`, the same target CI runs
#   7. verify            — the release is deployed and the public edge answers
#
# Step 3 is not politeness. The wipes are sequential: without it, an unreachable
# ClickHouse or a missing DROP grant is discovered AFTER MariaDB has already been
# dropped, which turns a refusal into a half-destroyed stand.
#
# SAFE TO RE-RUN. Every step is idempotent or absent-tolerant: uninstalling an
# absent release, dropping an absent database and deploying an already-current
# release are all no-ops that report themselves.
#
# PLAN BY DEFAULT. Nothing is destroyed without --apply.
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
#   --keep-databases       uninstall and redeploy, but do NOT drop any database.
#                          Rarely what you want — it is the state this script
#                          exists because of — but it is the smaller hammer.
#   --no-deploy            wipe only; do not reinstall. The stand is left EMPTY.
#   --timeout DURATION     helm timeout for the reinstall (default: 10m)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GITOPS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

ENV_NAME="test-stand"
KUBECONFIG_IN=""
CONTEXT=""
EXPECT_CLUSTER=""
CONFIRM=""
APPLY=0
KEEP_DATABASES=0
DO_DEPLOY=1
TIMEOUT="10m"

if [ -t 1 ] && [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'; C_CYA=$'\033[36m'; C_RST=$'\033[0m'
else
  C_RED=""; C_GRN=""; C_YEL=""; C_CYA=""; C_RST=""
fi

hdr()  { printf '\n%s── %s %s\n' "$C_CYA" "$1" "$C_RST" >&2; }
note() { printf '  %s\n' "$1" >&2; }
die()  { printf '%sERROR%s: %s\n' "$C_RED" "$C_RST" "$1" >&2; exit 1; }

usage() { sed -n '/^# Usage:/,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//; $d' >&2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --kubeconfig)     KUBECONFIG_IN="${2:?--kubeconfig needs a path}"; shift 2 ;;
    --context)        CONTEXT="${2:?--context needs a name}"; shift 2 ;;
    --expect-cluster) EXPECT_CLUSTER="${2:?--expect-cluster needs a name}"; shift 2 ;;
    --env)            ENV_NAME="${2:?--env needs a name}"; shift 2 ;;
    --confirm)        CONFIRM="${2:?--confirm needs a token}"; shift 2 ;;
    --timeout)        TIMEOUT="${2:?--timeout needs a duration}"; shift 2 ;;
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

# Images taken from the chart's OWN hook Jobs rather than chosen here, so the
# wipe never introduces a pull this stand does not already do. Kept in step with
# templates/{mariadb,clickhouse}-init-svcdbs-job.yaml.
MARIADB_IMAGE="docker.io/bitnamilegacy/mariadb:11.4.4-debian-12-r0"
CURL_IMAGE="curlimages/curl:8.10.1"

# Not helm-managed, must survive, and the stand cannot be rebuilt without them.
SURVIVORS_SECRET=(
  insight-db-creds
  insight-oidc
  insight-keycloak-admin
  insight-keycloak-config
  insight-authenticator-signing-keys
)
SURVIVORS_CM=("${RELEASE}-keycloak-config-realms")

KUBECTL=(kubectl)
[ -n "$KUBECONFIG_IN" ] && KUBECTL+=(--kubeconfig "$KUBECONFIG_IN")
[ -n "$CONTEXT" ] && KUBECTL+=(--context "$CONTEXT")
HELM=(helm)
[ -n "$KUBECONFIG_IN" ] && HELM+=(--kubeconfig "$KUBECONFIG_IN")
[ -n "$CONTEXT" ] && HELM+=(--kube-context "$CONTEXT")

# ── 1. Guards ──────────────────────────────────────────────────────────────
# The same shape as provision-ci-deployer.sh's target guard, and for a stronger
# reason: that script mints a credential, this one drops three databases.
#
# Compared on the CLUSTER the context resolves to, never on the context NAME. A
# context is a label in a file somebody wrote; two kubeconfigs on one laptop
# routinely name the same context differently and the same name differently.
hdr "target"

if [ -z "$CONTEXT" ]; then
  CONTEXT="$("${KUBECTL[@]}" config current-context 2>/dev/null || true)"
  [ -n "$CONTEXT" ] || die "no current-context and --context was not given"
  KUBECTL+=(--context "$CONTEXT")
  HELM+=(--kube-context "$CONTEXT")
fi

[ -n "$EXPECT_CLUSTER" ] || die "--expect-cluster is required. There is no default: this script destroys data, and the one thing it must never do is destroy somebody else's."

ACTUAL_CLUSTER="$("${KUBECTL[@]}" config view -o "jsonpath={.contexts[?(@.name==\"${CONTEXT}\")].context.cluster}" 2>/dev/null || true)"
[ -n "$ACTUAL_CLUSTER" ] || die "context '$CONTEXT' is not present in this kubeconfig"

if [ "$ACTUAL_CLUSTER" != "$EXPECT_CLUSTER" ]; then
  printf '%s\n' "$C_RED" >&2
  printf '  ┌──────────────────────────────────────────────────────────────┐\n' >&2
  printf '  │  REFUSING TO ACT — CLUSTER MISMATCH                          │\n' >&2
  printf '  └──────────────────────────────────────────────────────────────┘%s\n' "$C_RST" >&2
  printf '    context           : %s\n' "$CONTEXT" >&2
  printf '    resolves to       : %s\n' "$ACTUAL_CLUSTER" >&2
  printf '    --expect-cluster  : %s\n\n' "$EXPECT_CLUSTER" >&2
  printf '  Nothing was read, deleted or written.\n' >&2
  exit 2
fi
note "context   : $CONTEXT"
note "cluster   : $ACTUAL_CLUSTER"
note "namespace : $NAMESPACE"
note "release   : $RELEASE"
note "env       : $ENV_NAME"

"${KUBECTL[@]}" get namespace "$NAMESPACE" -o name >/dev/null 2>&1 \
  || die "namespace '$NAMESPACE' does not exist or is not readable with this credential"

EXPECT_CONFIRM="wipe-$ENV_NAME"
if [ "$APPLY" = "1" ] && [ "$CONFIRM" != "$EXPECT_CONFIRM" ]; then
  die "--apply requires --confirm $EXPECT_CONFIRM (a literal token, so it cannot be produced by a variable that happens to be wrong)"
fi

# ── 2. Survivors ───────────────────────────────────────────────────────────
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

# Presence is not enough for this one: the chart writes whatever it is given
# into the authenticator's config, so a Secret that exists with an empty value
# produces a confidential OIDC client with no secret — every pod Ready, the
# release `deployed`, and every login failing at the code exchange. The value is
# never printed; only its emptiness is.
if "${KUBECTL[@]}" -n "$NAMESPACE" get secret insight-oidc -o name >/dev/null 2>&1; then
  if [ -z "$("${KUBECTL[@]}" -n "$NAMESPACE" get secret insight-oidc -o "jsonpath={.data['client-secret']}" 2>/dev/null)" ]; then
    note "${C_RED}EMPTY${C_RST}   secret/insight-oidc has no 'client-secret' value"; missing=1
  else
    note "${C_GRN}ok${C_RST}    secret/insight-oidc carries a non-empty client-secret"
  fi
fi

if [ "$missing" = "1" ]; then
  die "refusing to wipe: something the rebuilt stand needs is absent. Restore it first — these objects are created by the bring-up outside this repository, not by the chart."
fi

# Helm-owned objects are listed for the reader's benefit: they are DESTROYED and
# REGENERATED, which is fine, and saying so here stops the next person wondering
# whether the uninstall lost something.
hdr "regenerated by the chart (destroyed on purpose)"
"${KUBECTL[@]}" -n "$NAMESPACE" get secret,configmap \
  -l app.kubernetes.io/managed-by=Helm -o name 2>/dev/null | sed 's/^/  /' >&2 || true

# ── 3. Datastore preflight ─────────────────────────────────────────────────
# READ-ONLY, and it runs before anything is destroyed. The two wipes below are
# sequential, so discovering an unreachable ClickHouse after MariaDB has been
# dropped would leave the stand in a state worse than the one being fixed.
hdr "datastore preflight (read-only)"

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
      containers:
        - name: preflight
          image: ${MARIADB_IMAGE}
          env:
            # secretKeyRef with EXPLICIT names, never envFrom. The keys in
            # insight-db-creds are hyphenated (\`mariadb-root-password\`), and
            # envFrom injects each key verbatim as a variable name — which is
            # not a valid shell identifier, so the script would run with the
            # passwords unset and every check would fail for the wrong reason.
            - {name: MYSQL_PWD,  valueFrom: {secretKeyRef: {name: insight-db-creds, key: mariadb-root-password}}}
            - {name: CH_PASSWORD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: clickhouse-password}}}
          command:
            - bash
            - -ec
            - |
              echo "MariaDB ${MARIADB_HOST}:${MARIADB_PORT} as root"
              mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot -e 'SELECT 1' >/dev/null
              for db in '${IDENTITY_DB}' '${KEYCLOAK_DB}' '${ANALYTICS_DB}'; do
                n=\$(mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot -N -B \\
                      -e "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name='\$db'")
                echo "  \$db: present=\$n"
              done
              echo "  root can DROP: \$(mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot -N -B -e "SHOW GRANTS FOR CURRENT_USER" | grep -c 'ALL PRIVILEGES ON \*\.\*' || true)"

              echo "ClickHouse ${CH_HOST}:${CH_PORT} as ${CH_USER}"
              code=\$(curl -s -o /tmp/ch -w '%{http_code}' \\
                --max-time 20 \\
                -H "X-ClickHouse-User: ${CH_USER}" \\
                -H "X-ClickHouse-Key: \$CH_PASSWORD" \\
                --data-binary 'SELECT count() FROM system.databases' \\
                'http://${CH_HOST}:${CH_PORT}/')
              [ "\$code" = "200" ] || { echo "ClickHouse answered \$code"; cat /tmp/ch; exit 1; }
              echo "  databases visible: \$(cat /tmp/ch)"
              echo "PREFLIGHT OK"
YAML
}

run_job() {
  local name="$1" manifest_fn="$2" timeout="$3"
  "$manifest_fn" | "${KUBECTL[@]}" apply -f - >/dev/null
  if ! "${KUBECTL[@]}" -n "$NAMESPACE" wait --for=condition=complete --timeout="$timeout" "job/$name" >/dev/null 2>&1; then
    "${KUBECTL[@]}" -n "$NAMESPACE" logs "job/$name" 2>&1 | sed 's/^/    /' >&2 || true
    "${KUBECTL[@]}" -n "$NAMESPACE" delete "job/$name" --ignore-not-found >/dev/null 2>&1 || true
    return 1
  fi
  "${KUBECTL[@]}" -n "$NAMESPACE" logs "job/$name" 2>&1 | sed 's/^/    /' >&2 || true
  "${KUBECTL[@]}" -n "$NAMESPACE" delete "job/$name" --ignore-not-found >/dev/null 2>&1 || true
  return 0
}

if [ "$APPLY" = "1" ]; then
  run_job "$PREFLIGHT_JOB" preflight_manifest 180s \
    || die "datastore preflight failed — nothing has been destroyed. Fix the connection or the grant and re-run."
  note "${C_GRN}preflight passed${C_RST}"
else
  note "would run a read-only Job asserting MariaDB and ClickHouse are reachable and authorised"
fi

# ── Plan ───────────────────────────────────────────────────────────────────
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
  note "   drop ClickHouse every application database (bronze_*, silver, insight,"
  note "   identity, staging, person, presentation, dbt_test__audit); the seed's"
  note "   create-bronze-placeholders.sh and the chart's migrate hook rebuild them"
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

# ── 4. Uninstall ───────────────────────────────────────────────────────────
hdr "uninstall"
if "${HELM[@]}" status "$RELEASE" -n "$NAMESPACE" >/dev/null 2>&1; then
  "${HELM[@]}" uninstall "$RELEASE" -n "$NAMESPACE" --wait --timeout 5m 2>&1 | sed 's/^/  /' >&2
else
  note "no release named $RELEASE — nothing to uninstall"
fi

# A FAILED hook Job is not deleted by its own hook-delete-policy (only a
# succeeded one is) and is not removed by uninstall either, so it would sit in
# the namespace and confuse the next run's diagnostics. Helm-labelled objects
# only: the survivors above carry no such label and are not matched.
"${KUBECTL[@]}" -n "$NAMESPACE" delete jobs -l app.kubernetes.io/managed-by=Helm --ignore-not-found >/dev/null 2>&1 || true

# ── 5. Database wipe ───────────────────────────────────────────────────────
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
      containers:
        - name: wipe
          image: ${MARIADB_IMAGE}
          env:
            - {name: MYSQL_PWD,   valueFrom: {secretKeyRef: {name: insight-db-creds, key: mariadb-root-password}}}
            - {name: CH_PASSWORD, valueFrom: {secretKeyRef: {name: insight-db-creds, key: clickhouse-password}}}
          command:
            - bash
            - -ec
            - |
              M() { mariadb -h'${MARIADB_HOST}' -P'${MARIADB_PORT}' -uroot "\$@"; }
              CH() {
                curl -sf --max-time 120 \\
                  -H "X-ClickHouse-User: ${CH_USER}" \\
                  -H "X-ClickHouse-Key: \$CH_PASSWORD" \\
                  --data-binary "\$1" 'http://${CH_HOST}:${CH_PORT}/'
              }

              echo "MariaDB: dropping ${IDENTITY_DB}, ${KEYCLOAK_DB}, ${ANALYTICS_DB}"
              M -e "DROP DATABASE IF EXISTS \\\`${IDENTITY_DB}\\\`"
              M -e "DROP DATABASE IF EXISTS \\\`${KEYCLOAK_DB}\\\`"
              M -e "DROP DATABASE IF EXISTS \\\`${ANALYTICS_DB}\\\`"

              # Recreated HERE, unlike the other two. The chart's
              # mariadb-init-svcdbs hook creates identity and keycloak on the
              # next install but never the analytics database, and the
              # operator's Database CR is cleanupPolicy: Skip — so nothing else
              # would bring it back. Charset and collation match that CR.
              echo "MariaDB: recreating ${ANALYTICS_DB} and granting ${DB_USER}"
              M -e "CREATE DATABASE \\\`${ANALYTICS_DB}\\\` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
              M -e "GRANT ALL PRIVILEGES ON \\\`${ANALYTICS_DB}\\\`.* TO \\\`${DB_USER}\\\`@\\\`%\\\`"
              M -e "FLUSH PRIVILEGES"

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

# ── 6. Deploy ──────────────────────────────────────────────────────────────
if [ "$DO_DEPLOY" = "1" ]; then
  hdr "deploy"
  VERSION="$(cat "$GITOPS_DIR/.insight-version" 2>/dev/null || true)"
  [ -n "$VERSION" ] || die "no version in $GITOPS_DIR/.insight-version — pass INSIGHT_VERSION to make deploy yourself"
  note "installing insight-$VERSION through the official target"
  make -C "$GITOPS_DIR" --no-print-directory deploy \
    ENV="$ENV_NAME" CONFIRM="yes-deploy-$ENV_NAME" \
    INSIGHT_VERSION="$VERSION" TIMEOUT="$TIMEOUT" 2>&1 | sed 's/^/  /' >&2

  # ── 7. Verify ────────────────────────────────────────────────────────────
  hdr "verify"
  make -C "$GITOPS_DIR" --no-print-directory verify-release \
    ENV="$ENV_NAME" INSIGHT_VERSION="$VERSION" 2>&1 | sed 's/^/  /' >&2

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

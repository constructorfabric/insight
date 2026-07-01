#!/usr/bin/env bash
#
# compose-app-secrets.sh — derive insight-analytics-api-config and
# insight-identity-config from the credentials already materialised in
# the cluster's `insight-db-creds` Secret, plus the L2 service hosts
# declared in environments/<env>/values.yaml.
#
# Why this exists: the chart auto-generates these "config" Secrets only
# when `credentials.autoGenerate: true`. The gitops contract forbids
# that combo (`gitops + autoGenerate=true` is blocked by the chart
# validator — rotation safety for ArgoCD reconciliation). The engineer
# is on the hook for creating the config Secrets in gitops mode.
#
# Rather than seal them as static manifests (which would need re-sealing
# every password rotation), we compose them at deploy time from the
# already-sealed `insight-db-creds`. Idempotent — `kubectl apply`
# overwrites on each run.
#
# Inputs (env vars):
#   ENV           required — selects environments/$ENV/values.yaml
#   NS_APP        required — namespace where the Secrets land (insight)
#   RELEASE       required — used to compute identity svc name
#
# The script reads from `environments/$ENV/values.yaml`:
#   .mariadb.host    .mariadb.port   .mariadb.username    .mariadb.database
#   .clickhouse.host .clickhouse.port .clickhouse.username .clickhouse.database
#   .redis.host      .redis.port
#   .identity.databaseName       (defaults to "identity")
#   .global.tenantDefaultId      (optional; empty disables the resolver
#                                 on both identity and analytics-api.
#                                 Single source of truth for the
#                                 single-tenant UUID — matches the
#                                 chart's `global.tenantDefaultId` knob
#                                 which also drives api-gateway's
#                                 single-tenant-tr-plugin.)
#   .identity.orgChartSourceType (optional; empty falls back to the
#                                 appsettings default `bamboohr`)
#
# Cleartext passwords live only in this shell's memory; they are never
# written to disk and never echoed.

set -euo pipefail

: "${ENV:?ENV is required}"
: "${NS_APP:?NS_APP is required}"
: "${RELEASE:?RELEASE is required}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VALUES="$ROOT/environments/$ENV/values.yaml"

[ -f "$VALUES" ] || { echo "ERROR: $VALUES not found" >&2; exit 1; }
command -v yq      >/dev/null || { echo "ERROR: yq is required" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "ERROR: kubectl is required" >&2; exit 1; }

# ── L2 connection coordinates (per-env, from values.yaml) ──
MDB_HOST=$(yq -r '.mariadb.host'             "$VALUES")
MDB_PORT=$(yq -r '.mariadb.port    // 3306'  "$VALUES")
MDB_USER=$(yq -r '.mariadb.username'         "$VALUES")
MDB_DB=$(  yq -r '.mariadb.database'         "$VALUES")
CH_HOST=$( yq -r '.clickhouse.host'          "$VALUES")
CH_PORT=$( yq -r '.clickhouse.port  // 8123' "$VALUES")
CH_USER=$( yq -r '.clickhouse.username'      "$VALUES")
CH_DB=$(   yq -r '.clickhouse.database'      "$VALUES")
RD_HOST=$( yq -r '.redis.host'               "$VALUES")
RD_PORT=$( yq -r '.redis.port       // 6379' "$VALUES")
IDENTITY_DB=$(yq -r '.identity.databaseName       // "identity"' "$VALUES")
TENANT_DEFAULT=$(yq -r '.global.tenantDefaultId          // ""' "$VALUES")
IDENTITY_ORG_CHART_SOURCE=$(yq -r '.identity.orgChartSourceType // ""' "$VALUES")

for v in MDB_HOST MDB_USER MDB_DB CH_HOST CH_USER CH_DB RD_HOST; do
  [ -n "${!v}" ] && [ "${!v}" != "null" ] || {
    echo "ERROR: $v not set in $VALUES" >&2
    exit 1
  }
done

# ── Passwords (from the controller-materialised insight-db-creds) ──
if ! kubectl -n "$NS_APP" get secret insight-db-creds >/dev/null 2>&1; then
  echo "ERROR: Secret $NS_APP/insight-db-creds not found." >&2
  echo "       Apply the L3 sealed manifests first:" >&2
  echo "         kubectl apply -f environments/$ENV/sealed-secrets/insight/" >&2
  echo "       Then wait a few seconds for the sealed-secrets-controller" >&2
  echo "       to decrypt before re-running." >&2
  exit 1
fi

MDB_PW=$(kubectl -n "$NS_APP" get secret insight-db-creds \
  -o jsonpath='{.data.mariadb-password}'   | base64 -d)
CH_PW=$( kubectl -n "$NS_APP" get secret insight-db-creds \
  -o jsonpath='{.data.clickhouse-password}'| base64 -d)
RD_PW=$( kubectl -n "$NS_APP" get secret insight-db-creds \
  -o jsonpath='{.data.redis-password}'     | base64 -d)

for v in MDB_PW CH_PW; do
  [ -n "${!v}" ] || {
    echo "ERROR: $v missing from $NS_APP/insight-db-creds — refusing to compose with empty password" >&2
    exit 1
  }
done

# Redis password is optional in principle; compose the URL without auth
# if it's blank, matching the chart's helper logic.
if [ -n "$RD_PW" ]; then
  REDIS_URL="redis://:${RD_PW}@${RD_HOST}:${RD_PORT}"
else
  REDIS_URL="redis://${RD_HOST}:${RD_PORT}"
fi

# ── Compose + apply ──
# kubectl apply -f - reads stdin; the YAML never lands on disk.
{
  cat <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: insight-analytics-api-config
  namespace: $NS_APP
  annotations:
    # Tell helm to leave this Secret alone on upgrade/uninstall — the
    # chart no longer emits it (credentials.autoGenerate=false in gitops
    # mode), and this script owns its lifecycle. Without `keep`, helm
    # sees the Secret in the prior release's manifest, finds it absent
    # from the new release's manifest, and deletes it mid-upgrade —
    # causing analytics-api init container to fail with "Secret not
    # found" and the upgrade to time out + roll back.
    helm.sh/resource-policy: keep
type: Opaque
stringData:
  # gears-rust host config: leaf values override the mounted config YAML.
  # Prefix is `APP__gears__analytics_api__config__*` (toolkit Env::prefixed),
  # replacing the old `ANALYTICS__*`.
  APP__gears__analytics_api__config__database_url: "mysql://${MDB_USER}:${MDB_PW}@${MDB_HOST}:${MDB_PORT}/${MDB_DB}"
  APP__gears__analytics_api__config__clickhouse_url: "http://${CH_HOST}:${CH_PORT}"
  APP__gears__analytics_api__config__clickhouse_database: "${CH_DB}"
  APP__gears__analytics_api__config__clickhouse_user: "${CH_USER}"
  APP__gears__analytics_api__config__clickhouse_password: "${CH_PW}"
  APP__gears__analytics_api__config__identity_url: "http://${RELEASE}-identity:8082"
  APP__gears__analytics_api__config__redis_url: "${REDIS_URL}"
EOF
  # Metric Catalog single-tenant fallback. Mirrors the chart-side block
  # (charts/insight/templates/secrets.yaml) — emit only when set so
  # multi-tenant installs keep the resolver strict.
  if [ -n "$TENANT_DEFAULT" ] && [ "$TENANT_DEFAULT" != "null" ]; then
    echo "  APP__gears__analytics_api__config__metric_catalog__tenant_default_id: \"${TENANT_DEFAULT}\""
  fi
} | kubectl -n "$NS_APP" apply -f - >/dev/null
echo "composed → $NS_APP/insight-analytics-api-config"

# `insight-identity-config` carries the .NET identity service's
# IDENTITY__* config. The service applies its own DbUp migrations
# against `${IDENTITY_DB}` at startup — see ADR-0006 (service-owned
# migrations). Empty IDENTITY__identity__tenant_default_id disables
# the config-default tenant resolver; callers must then send the
# X-Insight-Tenant-Id header.
{
  cat <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: insight-identity-config
  namespace: $NS_APP
  annotations:
    helm.sh/resource-policy: keep   # see analytics-api-config rationale above
type: Opaque
stringData:
  IDENTITY__mariadb__url: "mysql://${MDB_USER}:${MDB_PW}@${MDB_HOST}:${MDB_PORT}/${IDENTITY_DB}"
EOF
  if [ -n "$TENANT_DEFAULT" ] && [ "$TENANT_DEFAULT" != "null" ]; then
    echo "  IDENTITY__identity__tenant_default_id: \"${TENANT_DEFAULT}\""
  fi
  if [ -n "$IDENTITY_ORG_CHART_SOURCE" ] && [ "$IDENTITY_ORG_CHART_SOURCE" != "null" ]; then
    echo "  IDENTITY__identity__org_chart_source_type: \"${IDENTITY_ORG_CHART_SOURCE}\""
  fi
} | kubectl -n "$NS_APP" apply -f - >/dev/null
echo "composed → $NS_APP/insight-identity-config"

# Don't echo any of the passwords; clear the shell env explicitly.
unset MDB_PW CH_PW RD_PW REDIS_URL

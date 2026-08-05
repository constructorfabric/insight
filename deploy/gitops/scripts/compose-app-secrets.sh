#!/usr/bin/env bash
#
# compose-app-secrets.sh — derive the insight-{analytics,authenticator,
# identity-resolution}-config Secrets from the credentials already
# materialised in the cluster's `insight-db-creds` Secret, plus the L2
# service hosts declared in environments/<env>/values.yaml.
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
#   RELEASE       required — used to compute identity-resolution svc name
#
# The script reads from `environments/$ENV/values.yaml`:
#   .mariadb.host    .mariadb.port   .mariadb.username    .mariadb.database
#   .clickhouse.host .clickhouse.port .clickhouse.username .clickhouse.database
#   .redis.host      .redis.port
#   .identityResolution.databaseName (defaults to "identity")
#   .global.tenantDefaultId      (optional; empty disables the resolver
#                                 on both identity-resolution and
#                                 analytics. Single source of truth for
#                                 the single-tenant UUID — matches the
#                                 chart's `global.tenantDefaultId` knob.)
#   .identityUrl                 (optional; the identity URL ANALYTICS calls.
#                                 Empty = the default
#                                 `http://<release>-identity-resolution:8082`.
#                                 The authenticator does NOT use this — see below.)
#   .identityResolution.deploy   required to be true — the authenticator's
#                                 login-bootstrap resolve
#                                 (GET /internal/persons/by-external-id /
#                                 by-email-override) only exists on
#                                 identity-resolution (constructorfabric/insight#1960).
#                                 The authenticator is ALWAYS pointed at
#                                 `http://<release>-identity-resolution:8082`,
#                                 unlike analytics (overridable via
#                                 .identityUrl above).
#   .authenticator.oidc.sourceType       required — idp.source_type (the
#                                        identity-resolution source_type this
#                                        IdP is seeded under, e.g. "ms-entra").
#   .authenticator.oidc.externalIdClaim  optional; default "sub" (Entra: "oid").
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
TENANT_DEFAULT=$(yq -r '.global.tenantDefaultId          // ""' "$VALUES")
IDENTITY_RESOLUTION_BOOTSTRAP_ADMIN=$(yq -r '.identityResolution.bootstrapAdminPersonId // ""' "$VALUES")
IDENTITY_RESOLUTION_DB=$(yq -r '.identityResolution.databaseName // "identity"' "$VALUES")
# The identity URL ANALYTICS calls. Empty = the identity-resolution Service
# (constructorfabric/insight#1602). The AUTHENTICATOR does NOT use this —
# see AUTHENTICATOR_IDENTITY_URL below.
IDENTITY_URL=$(yq -r '.identityUrl // ""' "$VALUES")
[ -n "$IDENTITY_URL" ] && [ "$IDENTITY_URL" != "null" ] || IDENTITY_URL="http://${RELEASE}-identity-resolution:8082"

# The authenticator's login-bootstrap resolve
# (GET /internal/persons/by-external-id / by-email-override) only exists on
# identity-resolution (constructorfabric/insight#1960) — so, unlike analytics
# above, the authenticator is ALWAYS pointed at identity-resolution,
# regardless of .identityUrl. Refuse to compose a config that would point it
# at a service that was never deployed (helm's own render-time check —
# charts/insight/templates/_helpers.tpl `insight.validate` — covers the
# non-gitops path; this mirrors it for gitops installs, which skip that
# validator entirely since autoGenerate=false short-circuits the chart's own
# Secret rendering).
IDENTITY_RESOLUTION_DEPLOY=$(yq -r '.identityResolution.deploy // false' "$VALUES")
if [ "$IDENTITY_RESOLUTION_DEPLOY" != "true" ]; then
  echo "ERROR: identityResolution.deploy must be true in $VALUES — the authenticator's login-bootstrap resolve only exists on identity-resolution (constructorfabric/insight#1960)." >&2
  exit 1
fi
AUTHENTICATOR_IDENTITY_URL="http://${RELEASE}-identity-resolution:8082"

# ── Authenticator OIDC (NGINX_BFF). issuerUrl/redirectUri may be Helm template
#    strings in values.yaml; render {{ .Release.Name }}/{{ .Release.Namespace }}
#    the same way the chart's `tpl` would. ──
render_tpl() {
  # shellcheck disable=SC2001
  echo "$1" \
    | sed "s/{{[[:space:]]*\.Release\.Name[[:space:]]*}}/${RELEASE}/g" \
    | sed "s/{{[[:space:]]*\.Release\.Namespace[[:space:]]*}}/${NS_APP}/g"
}
AUTH_IDP_ISSUER=$(render_tpl "$(yq -r '.authenticator.oidc.issuerUrl   // ""' "$VALUES")")
AUTH_CLIENT_ID=$(          yq -r '.authenticator.oidc.clientId     // "insight-authenticator"' "$VALUES")
# Confidential-client secret: prefer the sealed `insight-oidc` Secret (Passbolt →
# seal-secret; never committed) and fall back to values.yaml (local/dev IdPs whose
# secret is not sensitive, e.g. the baked Keycloak dev client).
# If the env ships a sealed insight-oidc, wait for the controller to materialise it
# rather than silently composing an empty client secret on a fresh deploy.
if kubectl -n "$NS_APP" get sealedsecret insight-oidc >/dev/null 2>&1; then
  for i in $(seq 1 30); do
    kubectl -n "$NS_APP" get secret insight-oidc >/dev/null 2>&1 && break
    sleep 1
  done
  kubectl -n "$NS_APP" get secret insight-oidc >/dev/null 2>&1 || {
    echo "ERROR: sealed insight-oidc never materialised — refusing to compose an empty OIDC client secret" >&2
    exit 1
  }
fi
AUTH_CLIENT_SECRET=$(kubectl -n "$NS_APP" get secret insight-oidc \
  -o jsonpath='{.data.oidc-client-secret}' 2>/dev/null | base64 -d || true)
[ -n "$AUTH_CLIENT_SECRET" ] || AUTH_CLIENT_SECRET=$(yq -r '.authenticator.oidc.clientSecret // ""' "$VALUES")
AUTH_REDIRECT_URI=$(render_tpl "$(yq -r '.authenticator.oidc.redirectUri // ""' "$VALUES")")
# Requested OIDC scopes (space-delimited for the env layer; the authenticator
# splits it back into a list). Default matches the config default; an IdP that
# only issues a refresh token WITH offline_access (e.g. Entra) adds it here.
AUTH_SCOPES=$(yq -r '(.authenticator.oidc.scopes // ["openid","email","profile"]) | join(" ")' "$VALUES")
# Tenant sourcing: the id_token claim naming the single tenant (`tenant_id` on
# fakeidp/Keycloak, `tid` on Entra) and the fallback for a claim-less IdP
# (e.g. Okta). Empty fallback = fail closed downstream.
AUTH_TENANT_CLAIM=$(     yq -r '.authenticator.oidc.tenantClaim     // "tenant_id"' "$VALUES")
AUTH_DEFAULT_TENANT_ID=$(yq -r '.authenticator.oidc.defaultTenantId // ""' "$VALUES")
# The identity-resolution source_type this IdP is seeded under (e.g.
# "ms-entra") — required; drives the login-bootstrap resolve
# (GET /internal/persons/by-external-id?source_type=...&external_id=...).
AUTH_SOURCE_TYPE=$(yq -r '.authenticator.oidc.sourceType // ""' "$VALUES")
# id_token claim carrying the IdP's stable external user id for source_type
# (Entra: "oid"; the generic OIDC "sub" is not the same directory-stable id).
AUTH_EXTERNAL_ID_CLAIM=$(yq -r '.authenticator.oidc.externalIdClaim // "sub"' "$VALUES")
# `__override` view-as login (insight#1941/#1944) — dev/demo stands ONLY.
AUTH_OVERRIDE_ENABLED=$(yq -r '.authenticator.overrideEnabled // false' "$VALUES")
# The authn-tls discovery FQDN — the minted token `iss` and downstream issuer.
GATEWAY_ISSUER="https://${RELEASE}-authenticator.${NS_APP}.svc.cluster.local:8443"
GATEWAY_JWKS_URL="http://${RELEASE}-authenticator.${NS_APP}.svc.cluster.local:8083/.well-known/jwks.json"
AUTH_TOKEN_AUD="http://${RELEASE}-authenticator.${NS_APP}.svc.cluster.local:8093/internal/token"

for v in AUTH_IDP_ISSUER AUTH_REDIRECT_URI AUTH_SOURCE_TYPE; do
  [ -n "${!v}" ] && [ "${!v}" != "null" ] || {
    echo "ERROR: authenticator.oidc.* incomplete in $VALUES ($v empty) — auth is always on (NGINX_BFF); sourceType is required for the login-bootstrap resolve (constructorfabric/insight#1960)" >&2
    exit 1
  }
done

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
# Which ClickHouse user analytics connects as — the #1964 admin→read-only
# cutover switch, same contract as identityUrl above. Empty keeps the
# historical admin user, so environments pinned to a release without
# insight#2036 (which provisions the user) are untouched; per env, set
# .clickhouse.analyticsUsername to "presentation" once its pin carries #2036
# (the clickhouse-<user>-password key must be sealed into insight-db-creds).
# Rollback is clearing it back (+ rollout restart of analytics).
CH_ANALYTICS_USER=$(yq -r '.clickhouse.analyticsUsername // ""' "$VALUES")
[ "$CH_ANALYTICS_USER" != "null" ] || CH_ANALYTICS_USER=""
if [ -n "$CH_ANALYTICS_USER" ]; then
  CH_ANALYTICS_PW=$(kubectl -n "$NS_APP" get secret insight-db-creds \
    -o jsonpath="{.data.clickhouse-${CH_ANALYTICS_USER}-password}" | base64 -d)
else
  CH_ANALYTICS_USER="$CH_USER"
  CH_ANALYTICS_PW="$CH_PW"
fi

for v in MDB_PW CH_PW CH_ANALYTICS_PW; do
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
  name: insight-analytics-config
  namespace: $NS_APP
  annotations:
    # Tell helm to leave this Secret alone on upgrade/uninstall — the
    # chart no longer emits it (credentials.autoGenerate=false in gitops
    # mode), and this script owns its lifecycle. Without keep, helm
    # sees the Secret in the prior release's manifest, finds it absent
    # from the new release's manifest, and deletes it mid-upgrade —
    # causing analytics init container to fail with "Secret not
    # found" and the upgrade to time out + roll back.
    helm.sh/resource-policy: keep
type: Opaque
stringData:
  # gears-rust host config: leaf values override the mounted config YAML.
  # Prefix is APP__gears__analytics__config__ (toolkit Env::prefixed, gear
  # config key "analytics"). Note: no backticks in these heredoc comments --
  # the heredoc is unquoted (for \${..} expansion), so backticks would be
  # run as commands.
  APP__gears__analytics__config__database_url: "mysql://${MDB_USER}:${MDB_PW}@${MDB_HOST}:${MDB_PORT}/${MDB_DB}"
  APP__gears__analytics__config__clickhouse_url: "http://${CH_HOST}:${CH_PORT}"
  APP__gears__analytics__config__clickhouse_database: "${CH_DB}"
  APP__gears__analytics__config__clickhouse_user: "${CH_ANALYTICS_USER}"
  APP__gears__analytics__config__clickhouse_password: "${CH_ANALYTICS_PW}"
  APP__gears__analytics__config__identity_url: "${IDENTITY_URL}"
  APP__gears__analytics__config__redis_url: "${REDIS_URL}"
EOF
} | kubectl -n "$NS_APP" apply -f - >/dev/null
echo "composed → $NS_APP/insight-analytics-config"

# `insight-authenticator-config` (NGINX_BFF): the authenticator's leaf config.
# The chart emits this only when autoGenerate=true; in gitops mode we compose it
# here. redis reuses insight-db-creds; gateway_issuer is the authn-tls FQDN; the
# idp.* + redirect come from authenticator.oidc.* in values.yaml. The signing
# keys are a SEPARATE sealed Secret (insight-authenticator-signing-keys).
{
  cat <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: insight-authenticator-config
  namespace: $NS_APP
  annotations:
    helm.sh/resource-policy: keep   # see analytics-config rationale above
type: Opaque
stringData:
  APP__gears__authenticator__config__redis_url: "${REDIS_URL}"
  APP__gears__authenticator__config__identity_url: "${AUTHENTICATOR_IDENTITY_URL}"
  APP__gears__authenticator__config__gateway_issuer: "${GATEWAY_ISSUER}"
  APP__gears__authenticator__config__idp__issuer_url: "${AUTH_IDP_ISSUER}"
  APP__gears__authenticator__config__idp__client_id: "${AUTH_CLIENT_ID}"
  APP__gears__authenticator__config__idp__client_secret: "${AUTH_CLIENT_SECRET}"
  APP__gears__authenticator__config__idp__tenant_claim: "${AUTH_TENANT_CLAIM}"
  APP__gears__authenticator__config__idp__default_tenant_id: "${AUTH_DEFAULT_TENANT_ID}"
  APP__gears__authenticator__config__idp__source_type: "${AUTH_SOURCE_TYPE}"
  APP__gears__authenticator__config__idp__external_id_claim: "${AUTH_EXTERNAL_ID_CLAIM}"
  APP__gears__authenticator__config__redirect_uri: "${AUTH_REDIRECT_URI}"
  APP__gears__authenticator__config__oidc_scopes: "${AUTH_SCOPES}"
  APP__gears__authenticator__config__service_tokens__audience: "${AUTH_TOKEN_AUD}"
  APP__gears__authenticator__config__override_enabled: "${AUTH_OVERRIDE_ENABLED}"
EOF
} | kubectl -n "$NS_APP" apply -f - >/dev/null
echo "composed → $NS_APP/insight-authenticator-config"

# `insight-identity-resolution-config` carries the identity-resolution
# service's leaf config (gears-rust env-override convention, like analytics).
# It points at the MariaDB identity database the service owns and migrates,
# and reads ClickHouse over the HTTP protocol (8123) via the shared
# insight-clickhouse client.
{
  cat <<EOF
apiVersion: v1
kind: Secret
metadata:
  name: insight-identity-resolution-config
  namespace: $NS_APP
  annotations:
    helm.sh/resource-policy: keep   # see analytics-config rationale above
type: Opaque
stringData:
  APP__gears__identity_resolution__config__database_url: "mysql://${MDB_USER}:${MDB_PW}@${MDB_HOST}:${MDB_PORT}/${IDENTITY_RESOLUTION_DB}"
  APP__gears__identity_resolution__config__clickhouse_url: "http://${CH_HOST}:${CH_PORT}"
  APP__gears__identity_resolution__config__clickhouse_database: "${CH_DB}"
  APP__gears__identity_resolution__config__clickhouse_user: "${CH_USER}"
  APP__gears__identity_resolution__config__clickhouse_password: "${CH_PW}"
EOF
  # First-admin bootstrap inputs (migrate initContainer): mirror the
  # chart-side block in charts/insight/templates/secrets.yaml.
  if [ -n "$TENANT_DEFAULT" ] && [ "$TENANT_DEFAULT" != "null" ]; then
    echo "  APP__gears__identity_resolution__config__tenant_default_id: \"${TENANT_DEFAULT}\""
  fi
  if [ -n "$IDENTITY_RESOLUTION_BOOTSTRAP_ADMIN" ] && [ "$IDENTITY_RESOLUTION_BOOTSTRAP_ADMIN" != "null" ]; then
    echo "  APP__gears__identity_resolution__config__bootstrap_admin_person_id: \"${IDENTITY_RESOLUTION_BOOTSTRAP_ADMIN}\""
  fi
} | kubectl -n "$NS_APP" apply -f - >/dev/null
echo "composed → $NS_APP/insight-identity-resolution-config"

# Don't echo any of the passwords; clear the shell env explicitly.
unset MDB_PW CH_PW RD_PW REDIS_URL CH_ANALYTICS_PW

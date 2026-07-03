{{/*
==============================================================================
 Umbrella helpers
==============================================================================
Central place for release/component names (DRY) and service-reference
resolution. L2 infra (ClickHouse/MariaDB/Redis/Redpanda) is always
external — deployed out-of-chart at L2 — so each dep's `host`/`brokers`
field MUST be supplied; the helpers `required`-fail when it is empty.

Every fail-fast check lives in `insight.validate` at the bottom.
==============================================================================
*/}}

{{- define "insight.fullname" -}}
{{- default .Release.Name .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "insight.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: insight
{{- end -}}

{{/*
==============================================================================
 SERVICE RESOLUTION
==============================================================================
Contract per dep (all infra is external — out-of-chart L2):
  - `<dep>.host` — required (the helper `required`-fails when empty).
  - `<dep>.port` — required (has a value in values.yaml default).
  - `<dep>.url`  — composed "<scheme>://<host>:<port>" via helpers below.
  - `<dep>.fqdn` — the operator-supplied host verbatim.
==============================================================================
*/}}

{{/* ---------- ClickHouse ---------- *
     `host` resolution is fail-fast at the helper level (defense-in-depth):
     CH is external (out-of-chart L2), so `.host` MUST be supplied; we
     `required`-fail right here so any consumer that resolves the host
     before the validator template renders still gets a readable error
     rather than an empty/stale value. */}}
{{- define "insight.clickhouse.host" -}}
{{- required "clickhouse.host is required" .Values.clickhouse.host -}}
{{- end -}}

{{- define "insight.clickhouse.port" -}}
{{- required "clickhouse.port is required" .Values.clickhouse.port -}}
{{- end -}}

{{/* External CH: the FQDN is the operator-supplied host verbatim. */}}
{{- define "insight.clickhouse.fqdn" -}}
{{ include "insight.clickhouse.host" . }}
{{- end -}}

{{- define "insight.clickhouse.url" -}}
{{ include "insight.clickhouse.protocol" . }}://{{ include "insight.clickhouse.host" . }}:{{ include "insight.clickhouse.port" . }}
{{- end -}}

{{- define "insight.clickhouse.database" -}}
{{- required "clickhouse.database is required" .Values.clickhouse.database -}}
{{- end -}}

{{/* Wire protocol (http|https) for the Bronze ClickHouse destination.
     Defaults to plain HTTP (matching the http:// in insight.clickhouse.url
     above); override clickhouse.protocol for a TLS CH. */}}
{{- define "insight.clickhouse.protocol" -}}
{{- default "http" .Values.clickhouse.protocol -}}
{{- end -}}

{{/* ---------- MariaDB (external) ---------- */}}
{{- define "insight.mariadb.host" -}}
{{- required "mariadb.host is required" .Values.mariadb.host -}}
{{- end -}}

{{- define "insight.mariadb.port" -}}
{{- required "mariadb.port is required" .Values.mariadb.port -}}
{{- end -}}

{{- define "insight.mariadb.database" -}}
{{- required "mariadb.database is required" .Values.mariadb.database -}}
{{- end -}}

{{/* ---------- Redis (external) ---------- */}}
{{- define "insight.redis.host" -}}
{{- required "redis.host is required" .Values.redis.host -}}
{{- end -}}

{{- define "insight.redis.port" -}}
{{- required "redis.port is required" .Values.redis.port -}}
{{- end -}}

{{- define "insight.redis.url" -}}
redis://{{ include "insight.redis.host" . }}:{{ include "insight.redis.port" . }}
{{- end -}}

{{/* ---------- Redpanda (external) ----------
     The external Redpanda cluster's bootstrap brokers, as a single
     comma-separated host:port string (in-cluster clients use the
     internal listener, conventionally :9093).
*/}}
{{- define "insight.redpanda.brokers" -}}
{{- required "redpanda.brokers is required" .Values.redpanda.brokers -}}
{{- end -}}

{{/*
==============================================================================
 AIRBYTE (separate release, SAME namespace)
==============================================================================
*/}}
{{- define "insight.airbyte.url" -}}
{{- if .Values.airbyte.apiUrl -}}
{{- .Values.airbyte.apiUrl -}}
{{- else -}}
http://{{ .Values.airbyte.releaseName }}-airbyte-server-svc.{{ .Release.Namespace }}.svc.cluster.local:8001
{{- end -}}
{{- end -}}

{{/*
==============================================================================
 APP SERVICE HOSTS
==============================================================================
App services are mandatory umbrella components — no deploy flag.
*/}}
{{- define "insight.apiGateway.host"          -}}{{- printf "%s-api-gateway"          .Release.Name -}}{{- end -}}
{{- define "insight.analytics.host"           -}}{{- printf "%s-analytics"            .Release.Name -}}{{- end -}}
{{- define "insight.identity.host"            -}}{{- printf "%s-identity"             .Release.Name -}}{{- end -}}
{{- define "insight.frontend.host"            -}}{{- printf "%s-frontend"             .Release.Name -}}{{- end -}}

{{/*
==============================================================================
 VALIDATORS
==============================================================================
Fail-fast checks that run at helm template / install time.
Invoked from NOTES.txt so they fire on every install.
==============================================================================
*/}}
{{- define "insight.validate" -}}
  {{- /* GitOps + autoGenerate guard.
         Under ArgoCD/Flux, charts are rendered with `helm template` where
         Helm's `lookup` always returns nil. Combined with `autoGenerate=true`,
         this would regenerate `randAlphaNum 24` on every reconcile and rotate
         every DB password silently. There is no reliable in-chart way to
         detect the rendering tool, so we require the operator to declare
         the deployment mode explicitly and refuse the unsafe combination.
         Default is `helm` (imperative install); GitOps overlays MUST set
         `deploymentMode: gitops` AND `autoGenerate: false` together. */ -}}
  {{- $creds := default dict .Values.credentials -}}
  {{- $mode  := default "helm" $creds.deploymentMode -}}
  {{- if not (has $mode (list "helm" "gitops")) -}}
    {{- fail (printf "credentials.deploymentMode=%q is invalid; expected one of: helm, gitops" $mode) -}}
  {{- end -}}
  {{- if and (eq $mode "gitops") $creds.autoGenerate -}}
    {{- fail "credentials.deploymentMode=gitops is incompatible with credentials.autoGenerate=true. ArgoCD renders via `helm template` where `lookup` returns nil — auto-gen would rotate every DB password on each sync. Set credentials.autoGenerate: false and pre-create `insight-db-creds` (ExternalSecrets / sealed-secrets / SOPS)." -}}
  {{- end -}}

  {{- /* OIDC: when auth is enabled, require either existingSecret or ALL
         four inline fields. Defensive `default dict` guards against
         aggressive override files that remove the whole apiGateway /
         apiGateway.oidc block — without these, a nil-map dereference
         would replace the fail message with a cryptic template error.

         NB: `clientSecret` is intentionally NOT validated. The api-gateway
         uses Authorization Code + PKCE (public client flow) — `client_secret`
         has no meaning in this architecture. Operators with a Confidential
         IdP app should reconfigure it as Public/SPA-with-PKCE. */ -}}
  {{- $gw  := default dict .Values.apiGateway -}}
  {{- $oid := default dict $gw.oidc -}}
  {{- if not $gw.authDisabled -}}
    {{- if not $oid.existingSecret -}}
      {{- if or (not $oid.issuer) (not $oid.audience) (not $oid.clientId) (not $oid.redirectUri) -}}
        {{- fail "apiGateway.oidc: when existingSecret is empty and authDisabled=false, ALL of issuer + audience + clientId + redirectUri are required" -}}
      {{- end -}}
    {{- end -}}
  {{- end -}}

  {{- /* frontend.devUserEmail is the dev-impersonation escape hatch — the
         FE entrypoint stamps it into `oidc-config.js` so the browser
         builds an unsigned-JWT bearer on every /api/* call. Only
         meaningful when the gateway lets unauthenticated requests
         through (apiGateway.authDisabled=true). Setting it together
         with real auth is a misconfiguration — a forgotten value in a
         prod overlay would silently impersonate every visitor as that
         address. Catch it loudly at template time. */ -}}
  {{- $fe := default dict .Values.frontend -}}
  {{- if and $fe.devUserEmail (not $gw.authDisabled) -}}
    {{- fail (printf "frontend.devUserEmail (%q) is only valid when apiGateway.authDisabled=true. Either clear devUserEmail (real OIDC flow) or set apiGateway.authDisabled=true (sandbox dev impersonation)." $fe.devUserEmail) -}}
  {{- end -}}

  {{- /* External hosts (L2 infra is out-of-chart → consumer must supply
         host/brokers): the helper templates `insight.<dep>.host` and
         `insight.redpanda.brokers` already `required`-fail when empty, so
         any template that resolves the host before this validator runs
         gets a readable error. */ -}}

  {{- /* Passwords live in Secrets — never inline. Validate that the
         passwordSecret reference is present; the actual Secret may be
         auto-generated by the umbrella (credentials.autoGenerate=true),
         mirrored from a platform operator, or pre-created by the user. */ -}}
  {{- range $dep := list "clickhouse" "mariadb" "redis" -}}
    {{- $cfg := index $.Values $dep -}}
    {{- if not $cfg.passwordSecret.name -}}
      {{- fail (printf "%s.passwordSecret.name is required" $dep) -}}
    {{- end -}}
    {{- if not $cfg.passwordSecret.key -}}
      {{- fail (printf "%s.passwordSecret.key is required" $dep) -}}
    {{- end -}}
  {{- end -}}

  {{- /* BYO password hygiene. The MariaDB and Redis passwords are
         interpolated raw into DSNs (`mysql://insight:PASS@host:3306/db`,
         `redis://:PASS@host:6379`). Any of `@ : / ? # %` in PASS would
         silently break URL parsing — clients see a different host or a
         truncated password and fail at runtime, NOT at install. Auto-
         generated values come from `randAlphaNum` and are always safe;
         this check only fires when a pre-existing `insight-db-creds`
         Secret is found via `lookup` (BYO / Constructor Platform path).
         `helm template` returns nil from `lookup`, so the check is a
         no-op during local rendering. */ -}}
  {{- $dbSec := lookup "v1" "Secret" $.Release.Namespace "insight-db-creds" -}}
  {{- if $dbSec -}}
    {{- range $k := list "clickhouse-password" "mariadb-password" "mariadb-root-password" "redis-password" -}}
      {{- $raw := index $dbSec.data $k -}}
      {{- if $raw -}}
        {{- $val := $raw | b64dec -}}
        {{- if regexMatch "[@:/?#%]" $val -}}
          {{- fail (printf "insight-db-creds.%s contains a URL-reserved character ( @ : / ? # %% ). These silently corrupt the embedded DSN — clients parse the password at the first reserved char and fail at runtime, not at install. Use a password from [A-Za-z0-9._~-] only, or delete the Secret to let the umbrella auto-generate a safe one." $k) -}}
        {{- end -}}
      {{- end -}}
    {{- end -}}
  {{- end -}}
{{- end -}}

{{/*
==============================================================================
 Umbrella helpers
==============================================================================
Central place for release/component names (DRY) and service-reference
resolution. No separate `internal` vs `external` paths — each dep has a
single `host`/`port` field that either carries a default (empty → compute
from release name) or a user-supplied value (e.g. a Constructor Platform
hostname). The `deploy` flag only controls whether the umbrella itself
runs the dep as a subchart.

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
Contract per dep:
  - `<dep>.host` — if empty, defaults to the internal service name.
  - `<dep>.port` — required (has a value in values.yaml default).
  - `<dep>.url`  — composed "<scheme>://<host>:<port>" via helpers below.
  - `<dep>.fqdn` — fully-qualified DNS when the dep is internal, host
                   verbatim when external — useful for services that live
                   OUTSIDE the cluster but are resolved via kubelet.
==============================================================================
*/}}

{{/* ---------- ClickHouse ---------- *
     `host` resolution is fail-fast at the helper level (defense-in-depth):
     - deploy=true  → if .host is empty, default to `{release}-clickhouse`
                      (the bundled subchart Service name).
     - deploy=false → .host MUST be supplied; we `required`-fail right here
                      so any consumer that resolves the host before the
                      validator template renders still gets a readable
                      error rather than an empty/stale value. */}}
{{- define "insight.clickhouse.host" -}}
{{- if .Values.clickhouse.deploy -}}
{{- default (printf "%s-clickhouse" .Release.Name) .Values.clickhouse.host -}}
{{- else -}}
{{- required "clickhouse.host is required when clickhouse.deploy=false" .Values.clickhouse.host -}}
{{- end -}}
{{- end -}}

{{- define "insight.clickhouse.port" -}}
{{- required "clickhouse.port is required" .Values.clickhouse.port -}}
{{- end -}}

{{- define "insight.clickhouse.fqdn" -}}
{{- if .Values.clickhouse.deploy -}}
{{ include "insight.clickhouse.host" . }}.{{ .Release.Namespace }}.svc.cluster.local
{{- else -}}
{{ include "insight.clickhouse.host" . }}
{{- end -}}
{{- end -}}

{{- define "insight.clickhouse.url" -}}
http://{{ include "insight.clickhouse.host" . }}:{{ include "insight.clickhouse.port" . }}
{{- end -}}

{{- define "insight.clickhouse.database" -}}
{{- required "clickhouse.database is required" .Values.clickhouse.database -}}
{{- end -}}

{{/* Wire protocol (http|https) for the Bronze ClickHouse destination. The
     bundled ClickHouse serves plain HTTP on 8123, matching the http:// in
     insight.clickhouse.url above; override clickhouse.protocol for a TLS CH. */}}
{{- define "insight.clickhouse.protocol" -}}
{{- default "http" .Values.clickhouse.protocol -}}
{{- end -}}

{{/* ---------- MariaDB ---------- */}}
{{- define "insight.mariadb.host" -}}
{{- if .Values.mariadb.deploy -}}
{{- default (printf "%s-mariadb" .Release.Name) .Values.mariadb.host -}}
{{- else -}}
{{- required "mariadb.host is required when mariadb.deploy=false" .Values.mariadb.host -}}
{{- end -}}
{{- end -}}

{{- define "insight.mariadb.port" -}}
{{- required "mariadb.port is required" .Values.mariadb.port -}}
{{- end -}}

{{- define "insight.mariadb.database" -}}
{{- required "mariadb.database is required" .Values.mariadb.database -}}
{{- end -}}

{{/* ---------- Redis ---------- */}}
{{- define "insight.redis.host" -}}
{{- if .Values.redis.deploy -}}
{{- default (printf "%s-redis-master" .Release.Name) .Values.redis.host -}}
{{- else -}}
{{- required "redis.host is required when redis.deploy=false" .Values.redis.host -}}
{{- end -}}
{{- end -}}

{{- define "insight.redis.port" -}}
{{- required "redis.port is required" .Values.redis.port -}}
{{- end -}}

{{- define "insight.redis.url" -}}
redis://{{ include "insight.redis.host" . }}:{{ include "insight.redis.port" . }}
{{- end -}}

{{/* ---------- Redpanda ----------
     The Redpanda Helm chart exposes Kafka on two listeners:
       - 9093 — INTERNAL (in-cluster clients connect here)
       - 9092 — EXTERNAL (outside-cluster; goes through NodePort/LB)
     We resolve to the internal listener by default. Override via
     redpanda.brokers when pointing at an external cluster.
*/}}
{{- define "insight.redpanda.brokers" -}}
{{- if .Values.redpanda.deploy -}}
{{- default (printf "%s-redpanda:9093" .Release.Name) .Values.redpanda.brokers -}}
{{- else -}}
{{- required "redpanda.brokers is required when redpanda.deploy=false" .Values.redpanda.brokers -}}
{{- end -}}
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
{{- define "insight.analyticsApi.host"        -}}{{- printf "%s-analytics-api"        .Release.Name -}}{{- end -}}
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

  {{- /* External-mode hosts (deploy=false → consumer must supply host):
         the helper templates `insight.<dep>.host` already `required`-fail
         in this case, so any template that resolves the host before this
         validator runs gets a readable error. We keep the redpanda check
         here because `insight.redpanda.brokers` covers it the same way. */ -}}

  {{- /* MariaDB top-level vs subchart drift. The umbrella's secrets.yaml
         interpolates `mariadb.username` / `mariadb.database` into
         ANALYTICS__database_url; the bitnami subchart actually creates
         the user and database from `mariadb.auth.username` /
         `mariadb.auth.database`. values.yaml is not Helm-templated, so
         we cannot reference one from the other — operators must keep
         them in sync, and this validator catches drift loudly. */ -}}
  {{- if .Values.mariadb.deploy -}}
    {{- $mdb := .Values.mariadb -}}
    {{- $mdbAuth := default dict $mdb.auth -}}
    {{- if ne (default "" $mdb.username) (default "" $mdbAuth.username) -}}
      {{- fail (printf "mariadb.username (%q) and mariadb.auth.username (%q) must match. The umbrella uses the former for ANALYTICS__database_url; the bitnami subchart uses the latter to CREATE USER. Set both to the same value (or unset both)." $mdb.username $mdbAuth.username) -}}
    {{- end -}}
    {{- if ne (default "" $mdb.database) (default "" $mdbAuth.database) -}}
      {{- fail (printf "mariadb.database (%q) and mariadb.auth.database (%q) must match. Set both to the same value (or unset both)." $mdb.database $mdbAuth.database) -}}
    {{- end -}}
  {{- end -}}

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

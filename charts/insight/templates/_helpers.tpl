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

{{/* Topology of the target ClickHouse (epic #2010). One pair of values tells
     every creator in this release which DDL to emit: `clusterMode` turns on
     `Replicated*` engines, `clusterName` supplies the `ON CLUSTER <name>`
     clause. The two travel together — see requireNamedCluster below.
     Defaults render "false" and "", which is today's standalone DDL. */}}
{{- define "insight.clickhouse.clusterMode" -}}
{{- include "insight.clickhouse.requireNamedCluster" . -}}
{{- .Values.clickhouse.clusterMode | default false -}}
{{- end -}}

{{- define "insight.clickhouse.clusterName" -}}
{{- include "insight.clickhouse.requireNamedCluster" . -}}
{{- default "" .Values.clickhouse.clusterName -}}
{{- end -}}

{{/* Refuses the half-configured topology at render time, before anything in
     the release has created a relation: replicated engines whose DDL reaches
     only the node that answered the connection leave the other replicas
     without the table, which is the failure clusterMode exists to prevent.
     Checked here alone — every creator downstream is handed the pair by this
     chart and none of them is configured anywhere else.
     The name is an identifier every creator interpolates into DDL unquoted,
     the way dbt-clickhouse renders its own `cluster:` key, so what is accepted
     here is a bare ClickHouse identifier: a name carrying `-` or `.` parses
     only in quotes, and would be a syntax error in every statement it reached. */}}
{{- define "insight.clickhouse.requireNamedCluster" -}}
{{- if .Values.clickhouse.clusterMode -}}
{{- if not (regexMatch "^[A-Za-z_][A-Za-z0-9_]*$" (trim (default "" .Values.clickhouse.clusterName))) -}}
{{- fail (printf "clickhouse.clusterMode is true, so clickhouse.clusterName must name the cluster its DDL reaches, as a bare ClickHouse identifier — letters, digits and '_', starting with a letter or '_'. Got %q" .Values.clickhouse.clusterName) -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/* The environment a toolbox Job needs to issue ClickHouse DDL: the
     connection, the topology, and the password of every grant-less user
     create-databases.sh provisions. Shared by the two Jobs that run it — the
     pre-install `clickhouse-databases` Hook and the post-install
     `clickhouse-migrate` Hook, which reaches it through apply-ch-migrations.sh
     — so neither can drift into provisioning a different set of users. */}}
{{- define "insight.clickhouse.ddlEnv" -}}
# CLICKHOUSE_URL selects the HTTP backend in lib/ch-exec.sh.
- name: CLICKHOUSE_URL
  value: {{ include "insight.clickhouse.url" . | quote }}
- name: CLICKHOUSE_DATABASE
  value: {{ include "insight.clickhouse.database" . | quote }}
# Topology of the target ClickHouse (epic #2010) — what the DDL emitted here
# must be shaped for. Defaults render "false" / "", which is standalone.
- name: CLICKHOUSE_CLUSTER_MODE
  value: {{ include "insight.clickhouse.clusterMode" . | quote }}
- name: CLICKHOUSE_CLUSTER_NAME
  value: {{ include "insight.clickhouse.clusterName" . | quote }}
- name: CLICKHOUSE_USER
  value: {{ required "clickhouse.username is required" .Values.clickhouse.username | quote }}
- name: CLICKHOUSE_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ required "clickhouse.passwordSecret.name is required" .Values.clickhouse.passwordSecret.name | quote }}
      key:  {{ required "clickhouse.passwordSecret.key is required"  .Values.clickhouse.passwordSecret.key  | quote }}
# Grant-less `presentation` user's password (#1964). Optional: unset on a
# Secret predating this key -> the script provisions the role only.
- name: CLICKHOUSE_PRESENTATION_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ default "insight-db-creds" .Values.clickhouse.passwordSecret.name | quote }}
      key:  "clickhouse-presentation-password"
      optional: true
- name: MCP_ENABLED
  value: {{ .Values.global.mcp.enabled | quote }}
- name: SQL_API_ENABLED
  value: {{ .Values.global.sqlApi.enabled | quote }}
{{- if or .Values.global.mcp.enabled .Values.global.sqlApi.enabled }}
- name: CLICKHOUSE_MCP_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ required "analytics.mcp.credentialsSecret is required when MCP is enabled" .Values.analytics.mcp.credentialsSecret | quote }}
      key: {{ required "analytics.mcp.clickhousePasswordKey is required when MCP is enabled" .Values.analytics.mcp.clickhousePasswordKey | quote }}
{{- end }}
{{- if (((.Values.global).insightV3Core).deploy) }}
# Grant-less `insight_v3_reader`'s password. Optional: unset on a
# Secret predating this key -> the script provisions the role only,
# and the service queries as its own user.
- name: CLICKHOUSE_V3_READER_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ default "insight-db-creds" .Values.clickhouse.passwordSecret.name | quote }}
      key:  "clickhouse-v3-reader-password"
      optional: true
- name: CLICKHOUSE_V3_READER_USER
  value: {{ required "insightV3Core.clickhouseReaderUsername is required" .Values.insightV3Core.clickhouseReaderUsername | quote }}
{{- end }}
# Grant-less SELECT-only `grafana` user's password (#2888), shared
# with the Grafana datasource secret in insight-infra. Optional:
# unset -> the script provisions the role only.
- name: CLICKHOUSE_GRAFANA_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ default "insight-db-creds" .Values.clickhouse.passwordSecret.name | quote }}
      key:  "clickhouse-grafana-password"
      optional: true
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
 AIRBYTE (separate release; airbyte.namespace="" = same namespace as the app)
==============================================================================
*/}}
{{- define "insight.airbyte.namespace" -}}
{{- default .Release.Namespace .Values.airbyte.namespace -}}
{{- end -}}

{{- define "insight.airbyte.url" -}}
{{- if .Values.airbyte.apiUrl -}}
{{- .Values.airbyte.apiUrl -}}
{{- else -}}
http://{{ .Values.airbyte.releaseName }}-airbyte-server-svc.{{ include "insight.airbyte.namespace" . }}.svc.cluster.local:8001
{{- end -}}
{{- end -}}

{{/*
==============================================================================
 APP SERVICE HOSTS
==============================================================================
App services are mandatory umbrella components — no deploy flag.
*/}}
{{- define "insight.gateway.host"             -}}{{- printf "%s-gateway"              .Release.Name -}}{{- end -}}
{{- define "insight.authenticator.host"       -}}{{- printf "%s-authenticator"        .Release.Name -}}{{- end -}}
{{- define "insight.analytics.host"           -}}{{- printf "%s-analytics"            .Release.Name -}}{{- end -}}
{{- define "insight.identityResolution.host"  -}}{{- printf "%s-identity-resolution"  .Release.Name -}}{{- end -}}
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

  {{- /* Auth is ALWAYS on (NGINX_BFF EPIC #1583 — no auth_disabled path).
         Every request enters through the nginx `gateway`, which runs
         auth_request against the `authenticator`; the authenticator's OIDC
         upstream + browser callback are REQUIRED. Defensive `default dict`
         guards against override files that strip the whole authenticator
         block (a nil-map deref would mask the fail with a cryptic error).
         The leaf values are also `required` in templates/secrets.yaml; this
         is the earlier, friendlier message. */ -}}
  {{- $auth := default dict .Values.authenticator -}}
  {{- $aoidc := default dict $auth.oidc -}}
  {{- if or (not $aoidc.issuerUrl) (not $aoidc.redirectUri) -}}
    {{- fail "authenticator.oidc: issuerUrl (the IdP) and redirectUri (the browser callback) are REQUIRED — auth is always on (no auth_disabled). For local, point issuerUrl at the in-stack Keycloak realm URL and set keycloak.deploy=true." -}}
  {{- end -}}

  {{- /* The authenticator's login-bootstrap resolve
         (GET /internal/persons/by-external-id / by-email-override) exists on
         identity-resolution only (constructorfabric/insight#1960). Refuse to
         render a config that would point it at a service that was never
         deployed. */ -}}
  {{- if not (default dict .Values.identityResolution).deploy -}}
    {{- fail "identityResolution.deploy must be true — the authenticator's login-bootstrap resolve only exists on identity-resolution (constructorfabric/insight#1960)." -}}
  {{- end -}}
  {{- /* sourceType scopes the external-id resolve, so it is required only in
         that mode. The email mode resolves against the roster instead and
         reads neither knob — demanding one there would make an install name a
         source_type its login never asks about. */ -}}
  {{- $resolveBy := $aoidc.resolveBy | default "external_id" -}}
  {{- if not (has $resolveBy (list "external_id" "email")) -}}
    {{- fail (printf "authenticator.oidc.resolveBy must be \"external_id\" or \"email\", got %q" $resolveBy) -}}
  {{- end -}}
  {{- if and (eq $resolveBy "external_id") (not $aoidc.sourceType) -}}
    {{- fail "authenticator.oidc.sourceType is required — the identity-resolution source_type (e.g. \"ms-entra\") the login-bootstrap resolve is scoped to. Set authenticator.oidc.resolveBy=email instead if this IdP has no connector of its own and logins should resolve against the roster's addresses." -}}
  {{- end -}}
  {{- if eq $resolveBy "email" -}}
    {{- if not (default dict .Values.identityResolution).rosterSourceType -}}
      {{- fail "authenticator.oidc.resolveBy=email requires identityResolution.rosterSourceType — the email lookup is confined to the roster, and identity refuses it with no roster declared rather than matching an address any source happened to state. Every sign-in would be denied." -}}
    {{- end -}}
    {{- /* Minting needs the source-native id the roster observed, and an
           address is not one: no login provisions in this mode. The gear
           refuses the pair at boot; say so at render, where it is cheap. */ -}}
    {{- if $aoidc.provisionOnLogin -}}
      {{- fail "authenticator.oidc.provisionOnLogin cannot be used with resolveBy=email — minting needs the source-native id the roster observed, so no login provisions in this mode. Leave it false, or resolve by external_id." -}}
    {{- end -}}
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

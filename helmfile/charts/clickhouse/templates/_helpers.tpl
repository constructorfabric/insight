{{/*
Fullname helper. Previously the StatefulSet and Service used just
`{{ .Release.Name }}`, which under an umbrella chart produces a name
that collides with other resources bound to the same `{release}`
(for example, the frontend).

Now the resource name is `<release>-clickhouse`. This matches the
umbrella convention (`{release}-{component}`) and stays compatible
with helmfile, where the release name is set explicitly per release.

If `fullnameOverride` is set, it wins; otherwise `<release>-<chart-name>`.
*/}}
{{- define "clickhouse.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "clickhouse.labels" -}}
app.kubernetes.io/name: clickhouse
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "clickhouse.selectorLabels" -}}
app.kubernetes.io/name: clickhouse
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

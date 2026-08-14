{{/*
Every resource is named `preview-<experiment>` so many experiments coexist on
one host, and `helm uninstall preview-<experiment>` removes exactly one.
*/}}
{{- define "insight-preview.experiment" -}}
{{- $e := required "experiment is required (the /exp/<name> slug)" .Values.experiment -}}
{{- if not (regexMatch "^[a-z0-9]([-a-z0-9]*[a-z0-9])?$" $e) -}}
{{- fail (printf "experiment %q must be a DNS-1123 label: lowercase alphanumerics and '-', starting/ending alphanumeric" $e) -}}
{{- end -}}
{{- /* `preview-` prefix is 8 chars; cap at 55 so the resource name never
       trunc-collides two experiments sharing a 55-char prefix at the 63 limit. */ -}}
{{- if gt (len $e) 55 -}}
{{- fail (printf "experiment %q is too long: max 55 characters (the resource name preview-<experiment> must fit the 63-char limit)" $e) -}}
{{- end -}}
{{- $e -}}
{{- end }}

{{- define "insight-preview.fullname" -}}
{{- printf "preview-%s" (include "insight-preview.experiment" .) | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* The URL prefix this experiment is served under, e.g. /exp/<name>. */}}
{{- define "insight-preview.path" -}}
{{- printf "%s/%s" (trimSuffix "/" .Values.route.basePath) (include "insight-preview.experiment" .) -}}
{{- end }}

{{- define "insight-preview.labels" -}}
app.kubernetes.io/name: insight-preview
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
insight.dev/preview-experiment: {{ include "insight-preview.experiment" . }}
{{- end }}

{{- define "insight-preview.selectorLabels" -}}
app.kubernetes.io/name: insight-preview
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

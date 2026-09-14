{{- define "insight-v3-core.fullname" -}}
{{ .Release.Name }}-v3-core
{{- end }}

{{- define "insight-v3-core.labels" -}}
app.kubernetes.io/name: insight-v3-core
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-v3-core.selectorLabels" -}}
app.kubernetes.io/name: insight-v3-core
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

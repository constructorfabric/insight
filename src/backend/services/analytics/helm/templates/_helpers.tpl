{{- define "insight-analytics.fullname" -}}
{{ .Release.Name }}-analytics
{{- end }}

{{- define "insight-analytics.labels" -}}
app.kubernetes.io/name: analytics
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-analytics.selectorLabels" -}}
app.kubernetes.io/name: analytics
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

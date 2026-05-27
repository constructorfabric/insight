{{- define "claude-team-proxy.fullname" -}}
{{- if contains "claude-team-proxy" .Release.Name -}}
{{ .Release.Name }}
{{- else -}}
{{ .Release.Name }}-claude-team-proxy
{{- end -}}
{{- end }}

{{- define "claude-team-proxy.labels" -}}
app.kubernetes.io/name: claude-team-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: connector-proxy
{{- end }}

{{- define "claude-team-proxy.selectorLabels" -}}
app.kubernetes.io/name: claude-team-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-identity-resolution.fullname" -}}
{{ .Release.Name }}-identity-resolution
{{- end }}

{{- define "insight-identity-resolution.labels" -}}
app.kubernetes.io/name: identity-resolution
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-identity-resolution.selectorLabels" -}}
app.kubernetes.io/name: identity-resolution
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

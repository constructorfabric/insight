{{- define "insight-gateway.fullname" -}}
{{ .Release.Name }}-gateway
{{- end }}

{{- define "insight-gateway.labels" -}}
app.kubernetes.io/name: gateway
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-gateway.selectorLabels" -}}
app.kubernetes.io/name: gateway
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-keycloak.fullname" -}}
{{ .Release.Name }}-keycloak
{{- end }}

{{- define "insight-keycloak.labels" -}}
app.kubernetes.io/name: keycloak
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-keycloak.selectorLabels" -}}
app.kubernetes.io/name: keycloak
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-git-cli-proxy.fullname" -}}
{{ .Release.Name }}-git-cli-proxy
{{- end }}

{{- define "insight-git-cli-proxy.labels" -}}
app.kubernetes.io/name: git-cli-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-git-cli-proxy.selectorLabels" -}}
app.kubernetes.io/name: git-cli-proxy
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

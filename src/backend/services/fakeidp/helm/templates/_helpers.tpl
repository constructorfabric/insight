{{- define "insight-fakeidp.fullname" -}}
{{ .Release.Name }}-fakeidp
{{- end }}

{{- define "insight-fakeidp.labels" -}}
app.kubernetes.io/name: fakeidp
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-fakeidp.selectorLabels" -}}
app.kubernetes.io/name: fakeidp
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

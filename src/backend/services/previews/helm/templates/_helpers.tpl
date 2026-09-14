{{- define "insight-previews.fullname" -}}
{{ .Release.Name }}-previews
{{- end }}

{{- define "insight-previews.labels" -}}
app.kubernetes.io/name: previews
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-previews.selectorLabels" -}}
app.kubernetes.io/name: previews
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

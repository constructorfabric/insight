{{/*
Fullname = "<release>-frontend" rather than just "<release>".
Otherwise, under an umbrella chart (release = "insight") the resource
name collides with other resources that also bind to "insight".
*/}}
{{- define "insight-frontend.fullname" -}}
{{- printf "%s-frontend" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "insight-frontend.labels" -}}
app.kubernetes.io/name: insight-frontend
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "insight-frontend.selectorLabels" -}}
app.kubernetes.io/name: insight-frontend
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

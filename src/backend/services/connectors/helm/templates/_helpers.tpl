{{- define "insight-connectors.fullname" -}}
{{ .Release.Name }}-connectors
{{- end }}

{{- define "insight-connectors.labels" -}}
app.kubernetes.io/name: connectors
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-connectors.selectorLabels" -}}
app.kubernetes.io/name: connectors
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "insight-connectors.databaseEnv" -}}
- name: APP__gears__connectors__config__database_host
  value: {{ .Values.mariadb.host | quote }}
- name: APP__gears__connectors__config__database_port
  value: {{ .Values.mariadb.port | quote }}
- name: APP__gears__connectors__config__database_user
  value: {{ .Values.mariadb.username | quote }}
- name: APP__gears__connectors__config__database_name
  value: {{ .Values.databaseName | quote }}
- name: APP__gears__connectors__config__database_password
  valueFrom:
    secretKeyRef:
      name: {{ required "mariadb.passwordSecret.name is required" .Values.mariadb.passwordSecret.name | quote }}
      key: {{ required "mariadb.passwordSecret.key is required" .Values.mariadb.passwordSecret.key | quote }}
{{- end }}

{{- define "insight-connectors.readEnv" -}}
- name: APP__gears__connectors__config__clickhouse_url
  value: {{ printf "%s://%s:%v" .Values.clickhouse.protocol .Values.clickhouse.host .Values.clickhouse.port | quote }}
- name: APP__gears__connectors__config__clickhouse_database
  value: {{ .Values.clickhouse.database | quote }}
- name: APP__gears__connectors__config__clickhouse_user
  value: {{ .Values.clickhouse.username | quote }}
- name: APP__gears__connectors__config__clickhouse_password
  valueFrom:
    secretKeyRef:
      name: {{ required "clickhouse.passwordSecret.name is required" .Values.clickhouse.passwordSecret.name | quote }}
      key: {{ required "clickhouse.passwordSecret.key is required" .Values.clickhouse.passwordSecret.key | quote }}
- name: APP__gears__connectors__config__identity_url
  value: {{ tpl .Values.identityUrl . | quote }}
{{- end }}

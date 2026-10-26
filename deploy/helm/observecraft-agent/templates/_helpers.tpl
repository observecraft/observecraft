{{- define "agent.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 50 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 50 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "agent.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: observecraft
{{- end -}}

{{- define "agent.selector" -}}
{{- $ := index . 0 -}}
app.kubernetes.io/name: {{ $.Chart.Name }}
app.kubernetes.io/instance: {{ $.Release.Name }}
app.kubernetes.io/component: {{ index . 1 }}
{{- end -}}

{{- define "agent.image" -}}
{{ .Values.image.repository }}:{{ .Values.image.tag }}
{{- end -}}

{{- define "agent.secretName" -}}
{{ .Values.existingSecret | default (include "agent.fullname" .) }}
{{- end -}}

{{/* Env every agent needs. */}}
{{- define "agent.env" -}}
- name: OBSERVECRAFT_ENDPOINT
  value: {{ required "endpoint is required (the intake URL of your ObserveCraft install)" .Values.endpoint | trimSuffix "/" | quote }}
- name: OBSERVECRAFT_API_KEY
  valueFrom: { secretKeyRef: { name: {{ include "agent.secretName" . }}, key: api-key } }
- name: CLUSTER_NAME
  value: {{ required "clusterName is required" .Values.clusterName | quote }}
- name: K8S_NODE_NAME
  valueFrom: { fieldRef: { fieldPath: spec.nodeName } }
{{- end -}}

{{/* Export to the intake, with the queue on disk when a storage extension is configured. */}}
{{- define "agent.exporter" -}}
otlphttp/observecraft:
  endpoint: ${env:OBSERVECRAFT_ENDPOINT}
  headers:
    authorization: "Bearer ${env:OBSERVECRAFT_API_KEY}"
  compression: gzip
  retry_on_failure: { enabled: true, max_elapsed_time: 0 }
  sending_queue:
    enabled: true
    queue_size: 5000
    {{- if . }}
    storage: file_storage
    {{- end }}
{{- end -}}

{{- define "agent.securityContext" -}}
allowPrivilegeEscalation: false
readOnlyRootFilesystem: true
capabilities: { drop: [ALL] }
{{- end -}}

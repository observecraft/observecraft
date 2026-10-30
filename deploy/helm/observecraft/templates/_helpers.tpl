{{/* Names */}}
{{- define "oc.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 50 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 50 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "oc.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: observecraft
{{- end -}}

{{/* Selector labels of one component: include "oc.selector" (list . "server") */}}
{{- define "oc.selector" -}}
{{- $ := index . 0 -}}
app.kubernetes.io/name: {{ $.Chart.Name }}
app.kubernetes.io/instance: {{ $.Release.Name }}
app.kubernetes.io/component: {{ index . 1 }}
{{- end -}}

{{- define "oc.componentLabels" -}}
{{ include "oc.labels" (index . 0) }}
app.kubernetes.io/component: {{ index . 1 }}
{{- end -}}

{{/* Images */}}
{{- define "oc.registry" -}}
{{- with .Values.global.imageRegistry }}{{ . }}/{{ end -}}
{{- end -}}

{{- define "oc.image" -}}
{{ include "oc.registry" . }}{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}
{{- end -}}

{{/* include "oc.imageOf" (list . .Values.collector.image) */}}
{{- define "oc.imageOf" -}}
{{- $ := index . 0 -}}{{- $img := index . 1 -}}
{{ include "oc.registry" $ }}{{ $img.repository }}:{{ $img.tag }}
{{- end -}}

{{- define "oc.pullSecrets" -}}
{{- with .Values.global.imagePullSecrets }}
imagePullSecrets:
{{- range . }}
  - name: {{ . }}
{{- end }}
{{- end }}
{{- end -}}

{{- define "oc.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}{{ .Values.serviceAccount.name | default (include "oc.fullname" .) }}{{- else -}}{{ .Values.serviceAccount.name | default "default" }}{{- end -}}
{{- end -}}

{{- define "oc.storageClass" -}}
{{- with .Values.global.storageClass }}
storageClassName: {{ . | quote }}
{{- end }}
{{- end -}}

{{/* Secrets */}}
{{- define "oc.secretName" -}}
{{ .Values.secrets.existingSecret | default (printf "%s-secrets" (include "oc.fullname" .)) }}
{{- end -}}

{{/* A secret value: given in values, else the one already in the cluster, else generated (then kept:
    a new secret-key would make stored secrets unreadable). */}}
{{- define "oc.secretValue" -}}
{{- $old := index . 0 -}}{{- $key := index . 1 -}}{{- $given := index . 2 -}}
{{- if $given -}}{{ $given | b64enc }}{{- else if hasKey $old $key -}}{{ index $old $key }}{{- else -}}{{ randAlphaNum 40 | b64enc }}{{- end -}}
{{- end -}}

{{/* Endpoints */}}
{{- define "oc.publicUrl" -}}
{{- if .Values.publicUrl -}}{{ .Values.publicUrl | trimSuffix "/" }}
{{- else if .Values.ingress.enabled -}}{{ ternary "https" "http" .Values.ingress.tls }}://{{ .Values.ingress.app.host }}
{{- else -}}http://localhost:8080
{{- end -}}
{{- end -}}

{{- define "oc.clickhouseHost" -}}
{{- if eq .Values.clickhouse.mode "operator" -}}clickhouse-{{ include "oc.fullname" . }}
{{- else -}}{{ include "oc.fullname" . }}-clickhouse
{{- end -}}
{{- end -}}

{{- define "oc.clickhouseHttpUrl" -}}
{{- if eq .Values.clickhouse.mode "external" -}}{{ required "clickhouse.external.httpUrl is required with clickhouse.mode=external" .Values.clickhouse.external.httpUrl }}
{{- else -}}http://{{ include "oc.clickhouseHost" . }}:8123
{{- end -}}
{{- end -}}

{{- define "oc.clickhouseNative" -}}
{{- if eq .Values.clickhouse.mode "external" -}}{{ required "clickhouse.external.nativeAddress is required with clickhouse.mode=external" .Values.clickhouse.external.nativeAddress }}
{{- else -}}{{ include "oc.clickhouseHost" . }}:9000
{{- end -}}
{{- end -}}

{{- define "oc.clickhouseCluster" -}}
{{- if eq .Values.clickhouse.mode "operator" -}}{{ .Values.clickhouse.operator.cluster }}
{{- else if eq .Values.clickhouse.mode "external" -}}{{ .Values.clickhouse.external.cluster }}
{{- end -}}
{{- end -}}

{{- define "oc.clickhouseIngestUser" -}}
{{- if eq .Values.clickhouse.mode "external" -}}{{ .Values.clickhouse.external.ingestUser | default .Values.clickhouse.user }}
{{- else -}}{{ .Values.clickhouse.ingestUser }}
{{- end -}}
{{- end -}}

{{/* Secret key holding the ingest user's password. */}}
{{- define "oc.clickhouseIngestPasswordKey" -}}
{{- if and (eq .Values.clickhouse.mode "external") (not .Values.clickhouse.external.ingestUser) -}}clickhouse-password{{- else -}}clickhouse-ingest-password{{- end -}}
{{- end -}}

{{/* Kafka bootstrap brokers (host:port,…): the bundled cluster's service, or kafka.external.brokers. */}}
{{- define "oc.kafkaBrokers" -}}
{{- if .Values.kafka.enabled -}}{{ include "oc.fullname" . }}-kafka:9092{{- else -}}{{ required "kafka.external.brokers is required with kafka.enabled=false" .Values.kafka.external.brokers }}{{- end -}}
{{- end -}}

{{/* Secret holding the bundled Valkey's password ("" with cache.auth.enabled=false): cache.auth.existingSecret,
    else the chart's own secret. With secrets.existingSecret the chart generates nothing and can't see that
    secret's keys (helm template and Argo CD render without the cluster), so it asks instead of guessing: a
    missing key would leave Valkey and the server in CreateContainerConfigError. */}}
{{- define "oc.cacheAuthSecret" -}}
{{- $auth := .Values.cache.auth -}}
{{- if not $auth.enabled -}}
{{- else if $auth.existingSecret -}}{{ $auth.existingSecret }}
{{- else if .Values.secrets.existingSecret -}}
{{- fail (printf "secrets.existingSecret is set, so the chart doesn't generate the bundled Valkey's password. Add the key cache-password (letters and digits) to secret %s and set cache.auth.existingSecret=%s, or set cache.auth.existingSecret (and cache.auth.key) to another secret, or set cache.auth.enabled=false to run Valkey without a password (the NetworkPolicy still admits only the server). See \"Upgrading to 0.4.x\" in the chart README." .Values.secrets.existingSecret .Values.secrets.existingSecret) -}}
{{- else -}}{{ include "oc.secretName" . }}
{{- end -}}
{{- end -}}

{{- define "oc.cacheAuthKey" -}}
{{ .Values.cache.auth.key | default "cache-password" }}
{{- end -}}

{{/* OC_CACHE_URL and the cache settings for the server. */}}
{{- define "oc.cacheEnv" -}}
{{- if eq .Values.cache.mode "bundled" -}}
{{- with (include "oc.cacheAuthSecret" .) }}
- name: OC_CACHE_PASSWORD
  valueFrom: { secretKeyRef: { name: {{ . }}, key: {{ include "oc.cacheAuthKey" $ }} } }
- name: OC_CACHE_URL
  value: "redis://:$(OC_CACHE_PASSWORD)@{{ include "oc.fullname" $ }}-valkey:6379/0"
{{- else }}
- name: OC_CACHE_URL
  value: "redis://{{ include "oc.fullname" . }}-valkey:6379/0"
{{- end }}
{{- else -}}
- name: OC_CACHE_URL
  valueFrom: { secretKeyRef: { name: {{ include "oc.secretName" . }}, key: cache-url } }
{{- end }}
- { name: OC_CACHE_MAX_VALUE_BYTES, value: {{ .Values.cache.maxValueBytes | int64 | quote }} }
- { name: OC_CACHE_LOAD_TIMEOUT_SECONDS, value: {{ .Values.cache.loadTimeoutSeconds | quote }} }
{{- end -}}

{{/* Env shared by the server (and the migration step): databases and secrets. */}}
{{- define "oc.serverEnv" -}}
- name: OC_CLICKHOUSE_URL
  value: {{ include "oc.clickhouseHttpUrl" . | quote }}
- name: OC_CLICKHOUSE_USER
  value: {{ .Values.clickhouse.user | quote }}
- name: OC_CLICKHOUSE_PASSWORD
  valueFrom: { secretKeyRef: { name: {{ include "oc.secretName" . }}, key: clickhouse-password } }
{{- with (include "oc.clickhouseCluster" .) }}
- name: OC_CLICKHOUSE_CLUSTER
  value: {{ . | quote }}
{{- end }}
{{ include "oc.postgresEnv" . }}
{{- end -}}

{{/* OC_POSTGRES_URL for the server and intake. */}}
{{- define "oc.postgresEnv" -}}
{{- if eq .Values.postgres.mode "bundled" -}}
- name: OC_PG_PASSWORD
  valueFrom: { secretKeyRef: { name: {{ include "oc.secretName" . }}, key: postgres-password } }
- name: OC_POSTGRES_URL
  value: "postgres://{{ .Values.postgres.user }}:$(OC_PG_PASSWORD)@{{ include "oc.fullname" . }}-postgres:5432/{{ .Values.postgres.database }}"
{{- else if eq .Values.postgres.mode "operator" -}}
- name: OC_POSTGRES_URL
  valueFrom: { secretKeyRef: { name: {{ include "oc.fullname" . }}-postgres-app, key: uri } }
{{- else -}}
- name: OC_POSTGRES_URL
  valueFrom: { secretKeyRef: { name: {{ include "oc.secretName" . }}, key: postgres-url } }
{{- end -}}
{{- end -}}

{{- define "oc.podSecurity" -}}
securityContext:
  {{- toYaml .Values.podSecurityContext | nindent 2 }}
{{- end -}}

{{- define "oc.containerSecurity" -}}
securityContext:
  {{- toYaml .Values.containerSecurityContext | nindent 2 }}
{{- end -}}

{{/* Scheduling: include "oc.scheduling" (list . .Values.server "server") */}}
{{- define "oc.scheduling" -}}
{{- $ := index . 0 -}}{{- $v := index . 1 -}}{{- $component := index . 2 -}}
{{- with $v.nodeSelector }}
nodeSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $v.tolerations }}
tolerations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- if $v.affinity }}
affinity:
  {{- toYaml $v.affinity | nindent 2 }}
{{- else }}
# Spread replicas over nodes when possible.
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          topologyKey: kubernetes.io/hostname
          labelSelector:
            matchLabels:
              {{- include "oc.selector" (list $ $component) | nindent 14 }}
{{- end }}
{{- with $v.topologySpreadConstraints }}
topologySpreadConstraints:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}

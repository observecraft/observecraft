{{/* The cluster agent's collector config (before cluster.extraConfig is merged over it). */}}
{{- define "agent.clusterConfig" -}}
{{- $c := .Values.cluster -}}
{{- $i := $c.objectsInterval -}}
extensions:
  health_check:
    endpoint: 0.0.0.0:13133
  file_storage:
    directory: /var/lib/observecraft-agent
    create_directory: true

receivers:
  k8s_cluster:
    auth_type: serviceAccount
    collection_interval: {{ $c.metricsInterval }}
    node_conditions_to_report: [Ready, MemoryPressure, DiskPressure, PIDPressure, NetworkUnavailable]
    allocatable_types_to_report: [cpu, memory, pods]
    metrics:
      k8s.pod.status_reason: { enabled: true }
  k8sobjects:
    auth_type: serviceAccount
    objects:
      # Full objects: the Kubernetes Explorer's tables and YAML tab.
      {{- range list "pods" "nodes" "namespaces" "services" "persistentvolumeclaims" "persistentvolumes" "limitranges" "resourcequotas" "serviceaccounts" }}
      - { name: {{ . }}, mode: pull, interval: {{ $i }} }
      {{- end }}
      {{- range list "deployments" "replicasets" "daemonsets" "statefulsets" }}
      - { name: {{ . }}, group: apps, mode: pull, interval: {{ $i }} }
      {{- end }}
      - { name: jobs, group: batch, mode: pull, interval: {{ $i }} }
      - { name: cronjobs, group: batch, mode: pull, interval: {{ $i }} }
      - { name: horizontalpodautoscalers, group: autoscaling, mode: pull, interval: {{ $i }} }
      - { name: ingresses, group: networking.k8s.io, mode: pull, interval: {{ $i }} }
      - { name: networkpolicies, group: networking.k8s.io, mode: pull, interval: {{ $i }} }
      - { name: poddisruptionbudgets, group: policy, mode: pull, interval: {{ $i }} }
      {{- range list "roles" "rolebindings" "clusterroles" "clusterrolebindings" }}
      - { name: {{ . }}, group: rbac.authorization.k8s.io, mode: pull, interval: {{ $i }} }
      {{- end }}
      - { name: storageclasses, group: storage.k8s.io, mode: pull, interval: {{ $i }} }
      - { name: customresourcedefinitions, group: apiextensions.k8s.io, mode: pull, interval: {{ $i }} }
      {{- if $c.events }}
      # Kubernetes events as they happen (Events tab).
      - { name: events, group: events.k8s.io, mode: watch }
      {{- end }}

processors:
  memory_limiter: { check_interval: 1s, limit_percentage: 80, spike_limit_percentage: 20 }
  resource/cluster:
    attributes:
      - { key: k8s.cluster.name, value: "${env:CLUSTER_NAME}", action: upsert }
      - { key: observecraft.collector, value: observecraft-cluster-agent, action: upsert }
  # Marks object snapshots so ObserveCraft stores them as inventory, not as log lines.
  resource/objects:
    attributes:
      - { key: oc.signal, value: k8s.objects, action: upsert }
  batch: { send_batch_size: 1000, timeout: 5s }

exporters:
  {{- include "agent.exporter" true | nindent 2 }}

service:
  extensions: [health_check, file_storage]
  pipelines:
    metrics:
      receivers: [k8s_cluster]
      processors: [memory_limiter, resource/cluster, batch]
      exporters: [otlphttp/observecraft]
    logs/objects:
      receivers: [k8sobjects]
      processors: [memory_limiter, resource/cluster, resource/objects, batch]
      exporters: [otlphttp/observecraft]
{{- end }}

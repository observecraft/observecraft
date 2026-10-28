{{/* The node agent's collector config (before node.extraConfig is merged over it). */}}
{{- define "agent.nodeConfig" -}}
{{- $n := .Values.node -}}
extensions:
  health_check:
    endpoint: 0.0.0.0:13133
  file_storage:
    directory: /var/lib/observecraft-agent
    create_directory: true
  {{- if $n.prometheus.enabled }}
  k8s_observer:
    auth_type: serviceAccount
    node: ${env:K8S_NODE_NAME}
    observe_pods: true
  {{- end }}

receivers:
  {{- if $n.logs.enabled }}
  filelog:
    include: [/var/log/pods/*/*/*.log]
    exclude:
      # The agent's own logs would loop back.
      - /var/log/pods/{{ .Release.Namespace }}_{{ include "agent.fullname" . }}-*/*/*.log
      {{- range $n.logs.excludeNamespaces }}
      - /var/log/pods/{{ . }}_*/*/*.log
      {{- end }}
    start_at: end
    include_file_path: true
    # Read positions survive restarts: nothing skipped or sent twice.
    storage: file_storage
    operators:
      # CRI-O / containerd / Docker formats; sets k8s.namespace.name, k8s.pod.name, k8s.pod.uid, k8s.container.name.
      - type: container
        id: container-parser
  {{- end }}
  {{- if $n.kubeletMetrics.enabled }}
  kubeletstats:
    collection_interval: {{ $n.kubeletMetrics.interval }}
    auth_type: serviceAccount
    endpoint: https://${env:K8S_NODE_IP}:10250
    insecure_skip_verify: true
    metric_groups: [node, pod, container]
    metrics:
      k8s.container.cpu_limit_utilization: { enabled: true }
      k8s.container.cpu_request_utilization: { enabled: true }
      k8s.container.memory_limit_utilization: { enabled: true }
      k8s.container.memory_request_utilization: { enabled: true }
      k8s.pod.cpu_limit_utilization: { enabled: true }
      k8s.pod.memory_limit_utilization: { enabled: true }
  {{- end }}
  {{- if $n.hostMetrics.enabled }}
  hostmetrics:
    root_path: /hostfs
    collection_interval: {{ $n.hostMetrics.interval }}
    scrapers:
      cpu: { metrics: { system.cpu.utilization: { enabled: true } } }
      memory: { metrics: { system.memory.utilization: { enabled: true } } }
      load: {}
      # The node's real filesystems (container, kubelet and virtual mounts left out).
      filesystem:
        exclude_mount_points:
          mount_points: ["/dev/*", "/proc/*", "/sys/*", "/run/*", "/hostfs/*", "/etc/*", "/var/log/pods*", "/var/lib/observecraft-agent*", "/var/lib/docker/*", "/var/lib/kubelet/*", "/var/lib/containerd/*", "/snap/*"]
          match_type: regexp
        exclude_fs_types:
          fs_types: [autofs, binfmt_misc, bpf, cgroup2, configfs, debugfs, devpts, devtmpfs, fusectl, hugetlbfs, iso9660, mqueue, nsfs, overlay, proc, procfs, pstore, rpc_pipefs, securityfs, selinuxfs, squashfs, sysfs, tracefs]
          match_type: strict
      network: {}
  {{- end }}
  {{- if $n.otlp.enabled }}
  otlp:
    protocols:
      grpc: { endpoint: 0.0.0.0:4317 }
      http: { endpoint: 0.0.0.0:4318 }
  {{- end }}
  {{- if $n.datadog.traces }}
  datadog:
    endpoint: 0.0.0.0:8126
  {{- end }}
  {{- if $n.datadog.dogstatsd }}
  statsd:
    endpoint: 0.0.0.0:8125
    aggregation_interval: 10s
    enable_metric_type: true
    timer_histogram_mapping:
      - { statsd_type: histogram, observer_type: summary }
      - { statsd_type: timing, observer_type: summary }
      - { statsd_type: distribution, observer_type: summary }
  {{- end }}
  {{- if $n.prometheus.enabled }}
  # Autodiscovery: pods on this node annotated prometheus.io/scrape: "true".
  receiver_creator/prometheus:
    watch_observers: [k8s_observer]
    receivers:
      prometheus:
        rule: type == "pod" && annotations["prometheus.io/scrape"] == "true"
        config:
          config:
            scrape_configs:
              - job_name: autodiscovery
                scrape_interval: {{ $n.prometheus.interval }}
                metrics_path: '`"prometheus.io/path" in annotations ? annotations["prometheus.io/path"] : "/metrics"`'
                static_configs:
                  - targets: ['`endpoint`:`"prometheus.io/port" in annotations ? annotations["prometheus.io/port"] : "9090"`']
  {{- end }}

processors:
  memory_limiter: { check_interval: 1s, limit_percentage: 80, spike_limit_percentage: 20 }
  k8sattributes:
    auth_type: serviceAccount
    passthrough: false
    filter:
      node_from_env_var: K8S_NODE_NAME
    extract:
      metadata:
        - k8s.namespace.name
        - k8s.pod.name
        - k8s.pod.uid
        - k8s.pod.start_time
        - k8s.deployment.name
        - k8s.replicaset.name
        - k8s.daemonset.name
        - k8s.statefulset.name
        - k8s.job.name
        - k8s.cronjob.name
        - k8s.node.name
        - k8s.container.name
        - container.id
        - container.image.name
        - container.image.tag
      labels:
        # Unified service tagging (Datadog) and the standard app labels.
        - { tag_name: "k8s.pod.labels.tags.datadoghq.com/service", key: tags.datadoghq.com/service, from: pod }
        - { tag_name: "k8s.pod.labels.tags.datadoghq.com/env", key: tags.datadoghq.com/env, from: pod }
        - { tag_name: "k8s.pod.labels.tags.datadoghq.com/version", key: tags.datadoghq.com/version, from: pod }
        - { tag_name: "k8s.pod.labels.app.kubernetes.io/name", key: app.kubernetes.io/name, from: pod }
        - { tag_name: "k8s.pod.labels.app.kubernetes.io/instance", key: app.kubernetes.io/instance, from: pod }
        - { tag_name: "k8s.pod.labels.app.kubernetes.io/version", key: app.kubernetes.io/version, from: pod }
        - { tag_name: "k8s.pod.labels.app.kubernetes.io/component", key: app.kubernetes.io/component, from: pod }
        - { tag_name: "k8s.pod.labels.app.kubernetes.io/part-of", key: app.kubernetes.io/part-of, from: pod }
    pod_association:
      - sources: [{ from: resource_attribute, name: k8s.pod.uid }]
      - sources: [{ from: resource_attribute, name: k8s.pod.ip }]
      - sources: [{ from: connection }]
  resourcedetection:
    detectors: [env, system]
    override: false
    system: { hostname_sources: [os] }
  resource/cluster:
    attributes:
      - { key: k8s.cluster.name, value: "${env:CLUSTER_NAME}", action: upsert }
      - { key: k8s.node.name, value: "${env:K8S_NODE_NAME}", action: insert }
      - { key: host.name, value: "${env:K8S_NODE_NAME}", action: upsert }
      - { key: observecraft.collector, value: observecraft-agent, action: upsert }
      # Fleet Automation lists the node agents with this version.
      - { key: observecraft.collector.version, value: {{ printf "otelcol-contrib %s" .Values.image.tag | quote }}, action: upsert }
  batch: { send_batch_size: 2000, timeout: 2s }

exporters:
  {{- include "agent.exporter" true | nindent 2 }}

service:
  extensions: [health_check, file_storage{{ if $n.prometheus.enabled }}, k8s_observer{{ end }}]
  pipelines:
    {{- $logs := list }}{{ if $n.logs.enabled }}{{ $logs = append $logs "filelog" }}{{ end }}{{ if $n.otlp.enabled }}{{ $logs = append $logs "otlp" }}{{ end }}
    {{- if $logs }}
    logs:
      receivers: {{ toJson $logs }}
      processors: [memory_limiter, k8sattributes, resourcedetection, resource/cluster, batch]
      exporters: [otlphttp/observecraft]
    {{- end }}
    {{- $metrics := list }}
    {{- if $n.kubeletMetrics.enabled }}{{ $metrics = append $metrics "kubeletstats" }}{{ end }}
    {{- if $n.hostMetrics.enabled }}{{ $metrics = append $metrics "hostmetrics" }}{{ end }}
    {{- if $n.otlp.enabled }}{{ $metrics = append $metrics "otlp" }}{{ end }}
    {{- if $n.datadog.dogstatsd }}{{ $metrics = append $metrics "statsd" }}{{ end }}
    {{- if $n.prometheus.enabled }}{{ $metrics = append $metrics "receiver_creator/prometheus" }}{{ end }}
    {{- if $metrics }}
    metrics:
      receivers: {{ toJson $metrics }}
      processors: [memory_limiter, k8sattributes, resourcedetection, resource/cluster, batch]
      exporters: [otlphttp/observecraft]
    {{- end }}
    {{- $traces := list }}{{ if $n.otlp.enabled }}{{ $traces = append $traces "otlp" }}{{ end }}{{ if $n.datadog.traces }}{{ $traces = append $traces "datadog" }}{{ end }}
    {{- if $traces }}
    traces:
      receivers: {{ toJson $traces }}
      processors: [memory_limiter, k8sattributes, resourcedetection, resource/cluster, batch]
      exporters: [otlphttp/observecraft]
    {{- end }}
{{- end }}

// How GPU metrics reach ObserveCraft: the Datadog Agent (its GPU check or its DCGM check) or NVIDIA's
// DCGM exporter scraped by an OpenTelemetry Collector. {{intake}} and {{apiKey}} are filled in per install.
import { collectorConfig } from '../setup/collector';
import type { SetupOption } from '../setup/SetupOptions';

const COLLECTOR = collectorConfig(
  'gpu',
  `  prometheus/dcgm:
    config:
      scrape_configs:
        - job_name: dcgm-exporter
          scrape_interval: 30s
          static_configs:
            - targets: ['localhost:9400']   # or kubernetes_sd_configs for the exporter's pods`,
);

export const GPU_SETUP: SetupOption[] = [
  {
    id: 'datadog-gpu',
    name: 'Datadog Agent',
    summary: "The Agent's GPU check reads every NVIDIA GPU of its host through NVML and reports utilization, memory, temperature, power and errors as gpu.* metrics.",
    steps: [
      { title: 'Check the host', text: 'The NVIDIA driver must be installed (nvidia-smi works). Use a recent Agent 7 that already sends to ObserveCraft (Infrastructure → Add Resources).' },
      {
        title: 'Kubernetes: turn on GPU monitoring in the Datadog chart',
        text: 'Add this to the values file you already use for ObserveCraft and upgrade the release; the Agent pods on GPU nodes start the check.',
        code: 'datadog:\n  gpuMonitoring:\n    enabled: true',
      },
      {
        title: 'Host install: enable it in the Agent configuration',
        text: 'Set this in datadog.yaml (and system-probe.yaml for per-process usage), then restart the Agent.',
        code: 'gpu_monitoring:\n  enabled: true',
      },
      { title: 'Check the data', text: 'Within a minute the GPUs appear here, tagged with gpu_uuid, gpu_device (the model) and, on Kubernetes, the pods using them.' },
    ],
  },
  {
    id: 'datadog-dcgm',
    name: 'Datadog Agent + DCGM',
    summary: "If NVIDIA's DCGM exporter already runs on your GPU nodes, the Agent's dcgm check can scrape it and send dcgm.* metrics instead.",
    steps: [
      {
        title: 'Run the DCGM exporter',
        text: 'It listens on port 9400 (on Kubernetes, install it with its Helm chart).',
        code: 'docker run -d --gpus all --cap-add SYS_ADMIN -p 9400:9400 nvcr.io/nvidia/k8s/dcgm-exporter:<version>',
      },
      { title: 'Configure the check (conf.d/dcgm.d/conf.yaml)', code: 'instances:\n  - openmetrics_endpoint: http://localhost:9400/metrics' },
      { title: 'Restart the Agent', text: 'The GPUs appear here with the model and UUID labels of the exporter.' },
    ],
  },
  {
    id: 'otel-dcgm',
    name: 'DCGM exporter + OpenTelemetry',
    summary: 'No Datadog Agent: an OpenTelemetry Collector scrapes the DCGM exporter (DCGM_FI_DEV_* metrics) and sends them over OTLP.',
    steps: [
      {
        title: 'Run the DCGM exporter on every GPU node',
        text: 'On Kubernetes its Helm chart runs it as a DaemonSet and, with pod mapping on, labels each GPU with the pod, namespace and container using it.',
        code: 'helm repo add gpu-helm-charts https://nvidia.github.io/dcgm-exporter/helm-charts\nhelm install dcgm-exporter gpu-helm-charts/dcgm-exporter -n gpu-monitoring --create-namespace',
      },
      { title: 'Scrape it with the Collector', text: 'Add a Prometheus receiver for the exporter to the Collector that runs on the node (the observecraft-agent chart or your own).', code: COLLECTOR },
      { title: 'Check the data', text: 'GPUs are identified by the exporter’s UUID and Hostname labels; utilization, frame buffer memory, temperature and power fill the table.' },
    ],
  },
];

// How to run an Agent check against a workload on Kubernetes: the Datadog Agent's Autodiscovery annotations on the
// workload's pods (`ad.datadoghq.com/<container>.checks`), per check, for the ways the software is usually deployed.
// Our own words and examples; shown on the integration's Configure tab below the Agent configuration form.

export interface KubernetesHintVariant {
  /** How the software is deployed (its Helm chart, the Omnibus image). */
  label: string;
  /** Where the annotations go and what to set on the software's side. */
  text: string;
  /** The pod annotations (YAML). */
  annotations: string;
}

export interface KubernetesHint {
  check: string;
  variants: KubernetesHintVariant[];
}

/** `ad.datadoghq.com/<container>.checks` with the check's configuration, as pod annotations in YAML. */
export function checkAnnotation(container: string, check: string, config: { init_config?: object; instances: object[] }): string {
  const body = JSON.stringify({ [check]: { init_config: config.init_config ?? {}, instances: config.instances } }, null, 2);
  return `ad.datadoghq.com/${container}.checks: |\n${body
    .split('\n')
    .map((l) => `  ${l}`)
    .join('\n')}`;
}

/** The runner metrics the gitlab_runner check keeps (its sample configuration's list, shortened to the runner's own). */
const RUNNER_METRICS = [
  'ci_runner_builds',
  'ci_runner_errors',
  'gitlab_runner_api_request_duration_seconds',
  'gitlab_runner_autoscaling_machine_states',
  'gitlab_runner_errors_total',
  'gitlab_runner_job_queue_duration_seconds',
  'gitlab_runner_jobs',
  'gitlab_runner_version_info',
  'process_cpu_seconds_total',
  'process_resident_memory_bytes',
];

const HINTS: KubernetesHint[] = [
  {
    check: 'gitlab',
    variants: [
      {
        label: 'GitLab Helm chart (gitlab/gitlab)',
        text: 'Put the annotations on the webservice pods with the chart’s `gitlab.webservice.annotations` value; the chart already serves the Rails metrics on port 8083 (`gitlab.webservice.metrics.enabled`). Gitaly runs in its own pods: point `gitaly_server_endpoint` at its service. Replace `gitlab` in the service names with your release’s name.',
        annotations: checkAnnotation('webservice', 'gitlab', {
          instances: [
            {
              gitlab_url: 'http://%%host%%:8181',
              openmetrics_endpoint: 'http://%%host%%:8083/metrics',
              gitaly_server_endpoint: 'http://gitlab-gitaly.gitlab.svc:9236/metrics',
            },
          ],
        }),
      },
      {
        label: 'Omnibus image (gitlab/gitlab-ee) in a pod',
        text: "Annotate the pod running the Omnibus container (here named `gitlab`). Let the Agent read the metrics: add the pod network to `gitlab_rails['monitoring_whitelist']` and have Gitaly listen on all addresses (`gitaly['configuration'] = { prometheus_listen_addr: '0.0.0.0:9236' }`) in `GITLAB_OMNIBUS_CONFIG`, then restart GitLab.",
        annotations: checkAnnotation('gitlab', 'gitlab', {
          instances: [{ gitlab_url: 'http://%%host%%', openmetrics_endpoint: 'http://%%host%%/-/metrics', gitaly_server_endpoint: 'http://%%host%%:9236/metrics' }],
        }),
      },
    ],
  },
  {
    check: 'gitlab_runner',
    variants: [
      {
        label: 'GitLab Runner Helm chart (gitlab/gitlab-runner)',
        text: 'Turn the runner’s metrics on (`metrics.enabled: true`, port 9252) and put the annotations in the chart’s `podAnnotations` value. The container is named after the release (here `gitlab-runner`); `gitlab_url` is the GitLab the runner is registered with, for the check’s service check.',
        annotations: checkAnnotation('gitlab-runner', 'gitlab_runner', {
          init_config: { allowed_metrics: RUNNER_METRICS },
          instances: [{ prometheus_endpoint: 'http://%%host%%:9252/metrics', gitlab_url: 'https://gitlab.example.com' }],
        }),
      },
    ],
  },
];

/** The Kubernetes hint of an Agent check, if there is one. */
export function kubernetesHint(check: string): KubernetesHint | undefined {
  return HINTS.find((h) => h.check === check);
}

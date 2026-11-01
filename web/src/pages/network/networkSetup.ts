// How network connections reach ObserveCraft: the Datadog Agent's system-probe collects them and the
// process agent sends them to /api/v1/connections on this intake. {{intake}} is filled in per install.
import type { SetupOption } from '../setup/SetupOptions';

export const NETWORK_SETUP: SetupOption[] = [
  {
    id: 'kubernetes',
    name: 'Kubernetes (Datadog chart)',
    summary: "The chart's network monitoring runs system-probe next to each Agent; it records every TCP and UDP connection with its volume, retransmits and latency, plus DNS lookups.",
    steps: [
      { title: 'Check the cluster', text: 'The Agent already sends to ObserveCraft (Infrastructure → Add Resources). Nodes run Linux with a kernel of 4.4 or later.' },
      {
        title: 'Turn on network monitoring',
        text: 'Add this to the values file you already use for ObserveCraft and upgrade the release.',
        code: 'datadog:\n  networkMonitoring:\n    enabled: true\n  env:\n    - { name: DD_PROCESS_CONFIG_PROCESS_DD_URL, value: {{intake}} }',
      },
      { title: 'Check the data', text: 'After a minute the traffic between pods, services, hosts and availability zones appears here, and the DNS view lists the lookups.' },
    ],
  },
  {
    id: 'host',
    name: 'Host install',
    summary: 'On a VM or bare-metal host, enable the network module of system-probe and send the connections through the process agent.',
    steps: [
      { title: 'Enable the network module', text: 'In /etc/datadog-agent/system-probe.yaml:', code: 'network_config:\n  enabled: true' },
      { title: 'Send the connections to ObserveCraft', text: 'In /etc/datadog-agent/datadog.yaml, then restart the Agent and system-probe:', code: 'process_config:\n  process_dd_url: {{intake}}' },
      { title: 'Check the data', text: 'After a minute the host and its connections appear here.' },
    ],
  },
];

// The "Sending to ObserveCraft" steps of an integrations-core tile's Configure tab, by how Datadog collects it
// (collection.ts), in our own words. Tools that hand their logs to a local Datadog Agent (Supply Chain Firewall, IDE
// Shepherd) get their own: the Agent forwards them once its logs go to the ObserveCraft intake.
import type { CoreIntegration } from './catalog';

type Core = Pick<CoreIntegration, 'id' | 'check' | 'name' | 'collection' | 'customEndpoint' | 'configurable'>;

/** The Agent pointed at ObserveCraft, as docs/sending-data.md configures a host. */
const AGENT =
  'Install the Datadog Agent and point it at ObserveCraft in datadog.yaml: your ingest key as api_key, dd_url set to the intake, and for logs logs_enabled: true with logs_config.logs_dd_url (the intake as host:port) and logs_config.force_use_http: true.';

/** Tools that send their logs to the local Agent, by catalog id. */
const AGENT_LOG_SENDERS: Record<string, string[]> = {
  'supply-chain-firewall': [
    AGENT,
    'Supply Chain Firewall 3.x (pip install scfw): run scfw configure and answer yes to forwarding firewall logs to the local Datadog Agent. It writes conf.d/scfw.d/conf.yaml, a TCP listener (port 10365 by default) tagging the logs source:scfw and service:scfw. Choose the commands to log (allowed and blocked, or blocked only).',
    'Leave its API key option off: it sends to a Datadog site only and cannot be pointed at ObserveCraft.',
    'Restart the Agent (agent status lists scfw under Logs Agent) and click Install to add the log pipeline and the dashboard.',
    'Version 4 (the Go binary) no longer forwards logs to the Agent: it reports runs to Datadog Code Security and writes a local scfw.log in another format, which this pipeline and dashboard do not read.',
  ],
  'ide-shepherd': [
    AGENT,
    'In VS Code or Cursor, open the IDE Shepherd sidebar and turn Settings → Datadog Telemetry on. The extension writes conf.d/ide-shepherd.d/conf.yaml, a local listener tagging its events source:ide-shepherd, and offers to restart the Agent.',
    'Keep its default port. Click Install to add the log pipeline and the dashboard; the sidebar shows whether the Agent is reachable.',
  ],
};

/** How an integrations-core entry is set up to send to ObserveCraft. */
export function coreSteps(c: Core): string[] {
  const own = AGENT_LOG_SENDERS[c.id];
  if (own) return own;
  switch (c.collection) {
    case 'webhook':
      return [
        'Create an ingest key in Organization Settings → API Keys.',
        `In ${c.name}, add a webhook that posts to the URL below, with the ingest key in place of <INGEST_API_KEY>.`,
        'Click Install to add its log pipelines and dashboards.',
      ];
    case 'push-site':
      return c.customEndpoint
        ? [
            `In ${c.name}, choose a custom endpoint instead of a Datadog site and enter your ObserveCraft intake (below).`,
            'Use an ingest key from Organization Settings → API Keys where it asks for the Datadog API key.',
            'Click Install to add its log pipelines and dashboards.',
          ]
        : [];
    case 'crawler':
    case 'oauth':
    case 'docs':
      return [];
    default:
      return [
        AGENT,
        c.configurable
          ? `Fill in the settings below and save the result as conf.d/${c.check}.d/conf.yaml in the Agent's configuration directory.`
          : `Configure the ${c.name} integration as its integrations-core documentation describes.`,
        'Restart the Agent. Data appears within a few minutes; detection uses metrics received during the last 24 hours.',
      ];
  }
}

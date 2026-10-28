import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, GitCompare, Info } from 'lucide-react';
import type { FleetProfile } from '../../api/fleet/types';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { diffStats, kindLabel, lineDiff } from './fleetModel';
import { IntegrationLogos, KindIcon } from './fleetUi';
import { useFleetAgent, useFleetProfiles } from './queries';

/** Where configuration is changed, per deployment (ObserveCraft doesn't push configuration). */
const HOW_TO: { title: string; text: string; filename: string; code: string }[] = [
  {
    title: 'Datadog Helm chart',
    text: 'Set the Agent options in your values file and upgrade the release; the DaemonSet rolls out with the new configuration.',
    filename: 'values.yaml',
    code: 'datadog:\n  logs:\n    enabled: true\n  apm:\n    portEnabled: true\n  env:\n    - name: DD_INVENTORIES_CONFIGURATION_ENABLED\n      value: "true"\n\n# helm upgrade <release> datadog/datadog -f values.yaml',
  },
  {
    title: 'datadog.yaml on a host',
    text: 'Edit the main configuration file (and conf.d/<check>.d/conf.yaml for integrations), then restart the Agent.',
    filename: '/etc/datadog-agent/datadog.yaml',
    code: 'logs_enabled: true\napm_config:\n  enabled: true\n# Report the full configuration to Fleet Automation\ninventories_configuration_enabled: true\n\n# sudo systemctl restart datadog-agent',
  },
  {
    title: 'Datadog Operator',
    text: 'Change the DatadogAgent resource; the Operator applies it to every node.',
    filename: 'datadog-agent.yaml',
    code: 'spec:\n  features:\n    logCollection:\n      enabled: true\n    apm:\n      enabled: true',
  },
  {
    title: 'observecraft-agent chart',
    text: 'The OpenTelemetry Collectors take their pipelines from the chart values (node.*, cluster.*, and extraConfig for anything else).',
    filename: 'values.yaml',
    code: 'node:\n  logs:\n    excludeNamespaces: [kube-system]\n  hostMetrics:\n    interval: 30s\n\n# helm upgrade observecraft-agent oci://ghcr.io/observecraft/charts/observecraft-agent -f values.yaml',
  },
];

/** Settings shown on a profile's summary line. */
const KEY_SETTINGS: [string, string][] = [
  ['install-method.tool', 'install'],
  ['install_method', 'install'],
  ['feature_logs_enabled', 'logs'],
  ['feature_apm_enabled', 'APM'],
  ['feature_process_enabled', 'processes'],
];

function settingChips(p: FleetProfile): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const [key, label] of KEY_SETTINGS) {
    const v = p.settings[key];
    if (v == null || seen.has(label)) continue;
    seen.add(label);
    out.push(v === 'true' ? `${label} on` : v === 'false' ? `${label} off` : `${label}: ${v}`);
  }
  return out;
}

/** Integrations → Fleet Automation → Configuration: the configurations running in the fleet, read-only. */
export function ConfigurationPage() {
  const profiles = useFleetProfiles();
  return (
    <div className="fl-view">
      <div className="fl-banner">
        <Info size={16} />
        <div>
          <strong>Read-only: configurations are changed where agents are deployed.</strong> ObserveCraft shows the configuration agents report but doesn’t implement remote configuration, so nothing
          here is pushed to an agent. Use the deployment the agent came from:
        </div>
      </div>
      <HowTo />
      {profiles.isError ? (
        <ErrorState error={profiles.error} onRetry={() => profiles.refetch()} />
      ) : !profiles.data ? (
        <Loading label="Loading configurations…" />
      ) : profiles.data.length === 0 ? (
        <p className="muted">No agents reported in this time range.</p>
      ) : (
        <>
          <Profiles profiles={profiles.data} />
          <Compare profiles={profiles.data} />
        </>
      )}
    </div>
  );
}

function HowTo() {
  const [tab, setTab] = useState(0);
  const t = HOW_TO[tab];
  return (
    <section className="fl-section fl-howto">
      <div className="fl-tabs" role="tablist">
        {HOW_TO.map((h, i) => (
          <button key={h.title} role="tab" aria-selected={i === tab} className={`fl-tab${i === tab ? ' active' : ''}`} onClick={() => setTab(i)}>
            {h.title}
          </button>
        ))}
      </div>
      <p>{t.text}</p>
      <CodeBlock code={t.code} filename={t.filename} />
    </section>
  );
}

function Profiles({ profiles }: { profiles: FleetProfile[] }) {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <section className="fl-section">
      <h2 className="fl-h2">
        {profiles.length} {profiles.length === 1 ? 'configuration' : 'configurations'} in the fleet
      </h2>
      <p className="muted">Agents of the same type running the same integrations with the same settings (secrets redacted, versions left out) share a configuration.</p>
      <div className="fl-profiles">
        {profiles.map((p) => (
          <div key={p.id} className="fl-profile">
            <button className="fl-profile-row" onClick={() => setOpen((o) => (o === p.id ? null : p.id))} aria-expanded={open === p.id}>
              {open === p.id ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <KindIcon kind={p.kind} />
              <span className="fl-profile-name">
                {kindLabel(p.kind)} <span className="mono muted">#{p.id}</span>
              </span>
              <span className="fl-profile-count">
                {p.agents.length} {p.agents.length === 1 ? 'agent' : 'agents'}
              </span>
              <IntegrationLogos integrations={p.integrations} max={6} />
              <span className="fl-chips">
                {settingChips(p).map((c) => (
                  <span key={c} className="tag-chip">
                    {c}
                  </span>
                ))}
              </span>
              <span className="muted fl-profile-versions">{p.versions.join(', ')}</span>
            </button>
            {open === p.id && (
              <div className="fl-profile-body">
                <div>
                  <h4>Agents</h4>
                  <ul className="fl-plain">
                    {p.agents.map((a) => (
                      <li key={a.id}>
                        <span className="mono">{a.hostname}</span> <span className="muted">{a.version ?? 'unknown version'}</span>
                      </li>
                    ))}
                  </ul>
                </div>
                <div>
                  <h4>Settings</h4>
                  {Object.keys(p.settings).length === 0 ? (
                    <p className="muted">None reported.</p>
                  ) : (
                    <table className="fl-table fl-table-compact">
                      <tbody>
                        {Object.entries(p.settings).map(([k, v]) => (
                          <tr key={k}>
                            <td className="mono">{k}</td>
                            <td className="mono">{v}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

function Compare({ profiles }: { profiles: FleetProfile[] }) {
  const agents = useMemo(() => profiles.flatMap((p) => p.agents.map((a) => ({ ...a, profile: p.id, kind: p.kind }))), [profiles]);
  const options = agents.map((a) => ({ value: a.id, label: `${a.hostname} (${kindLabel(a.kind)} ${a.version ?? ''}, #${a.profile})` }));
  // Defaults: one agent of each of the two largest configurations.
  const [left, setLeft] = useState(profiles[0]?.agents[0]?.id ?? '');
  const [right, setRight] = useState((profiles[1] ?? profiles[0])?.agents[profiles[1] ? 0 : 1]?.id ?? profiles[0]?.agents[0]?.id ?? '');
  const a = useFleetAgent(left || null);
  const b = useFleetAgent(right || null);
  const diff = useMemo(() => (a.data && b.data ? lineDiff(a.data.configYaml ?? '', b.data.configYaml ?? '') : null), [a.data, b.data]);
  const stats = diff ? diffStats(diff) : null;
  return (
    <section className="fl-section">
      <h2 className="fl-h2">
        <GitCompare size={16} /> Compare two agents
      </h2>
      <div className="fl-compare-pick">
        <Select value={left} options={options} onChange={setLeft} title="Left agent" />
        <span className="muted">with</span>
        <Select value={right} options={options} onChange={setRight} title="Right agent" />
        {stats && (
          <span className="fl-diff-stats">
            <span className="fl-add">+{stats.added}</span> <span className="fl-del">−{stats.removed}</span>
          </span>
        )}
      </div>
      {a.isError || b.isError ? (
        <ErrorState error={a.error ?? b.error} />
      ) : !diff ? (
        <Loading label="Loading configurations…" />
      ) : !a.data?.configYaml && !b.data?.configYaml ? (
        <p className="muted">Neither agent reports its configuration.</p>
      ) : (
        <div className="fl-diff" role="table" aria-label="Configuration differences">
          {diff.map((d, i) => (
            <div key={i} className={`fl-diff-line fl-diff-${d.kind}`} role="row">
              <span className="fl-diff-no">{d.left ?? ''}</span>
              <span className="fl-diff-no">{d.right ?? ''}</span>
              <span className="fl-diff-sign">{d.kind === 'add' ? '+' : d.kind === 'del' ? '−' : ' '}</span>
              <span className="fl-diff-text">{d.text || ' '}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

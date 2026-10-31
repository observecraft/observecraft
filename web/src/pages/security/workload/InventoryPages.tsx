import { Fragment, useMemo, useState, type ReactNode } from 'react';
import { Bug, ChevronDown, ChevronRight, ClipboardCheck, Download, Layers, PanelLeftClose, PanelLeftOpen, Plus, RefreshCw, Search, Server, Settings2, ShieldCheck, Wrench, X } from 'lucide-react';
import type { CsmAgent } from '../../../api/workloadProtection/client';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { Pager } from '../../../components/ui/Pager';
import { BrandLogo, hasBrandLogo } from '../../../components/brand/BrandLogo';
import { timeAgo } from '../../../lib/format';
import { INVENTORY_TABS, WpTabs } from './WorkloadLayout';
import { useWpAgents } from './manageQueries';
import './manage.css';

// ---------------------------------------------------------------- facets (shared with the Policies pages)

export interface FacetDef<T> {
  key: string;
  label: string;
  /** Section header the facet sits under (HOST, AGENT, CORE, …). */
  group: string;
  value: (row: T) => string | string[];
  open?: boolean;
}
export type FacetSel = Record<string, Set<string>>;

const valuesOf = <T,>(def: FacetDef<T>, row: T) => {
  const v = def.value(row);
  return (Array.isArray(v) ? v : [v]).filter((x) => x !== '');
};

/** Rows that pass every facet; an empty selection lets everything through. */
export function applyFacets<T>(rows: T[], defs: FacetDef<T>[], sel: FacetSel): T[] {
  return rows.filter((r) =>
    defs.every((d) => {
      const s = sel[d.key];
      if (!s || !s.size) return true;
      const vs = valuesOf(d, r);
      return vs.length ? vs.some((v) => s.has(v)) : s.has('');
    }),
  );
}

/** The left facet column: collapsible facets under grey section headers, every value checked by default. */
export function FacetPanel<T>({ rows, defs, sel, onChange, top }: { rows: T[]; defs: FacetDef<T>[]; sel: FacetSel; onChange: (s: FacetSel) => void; top?: ReactNode }) {
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState<Record<string, boolean>>(() => Object.fromEntries(defs.map((d) => [d.key, !!d.open])));
  const [hidden, setHidden] = useState<Record<string, boolean>>({});
  const shown = defs.filter((d) => !search.trim() || d.label.toLowerCase().includes(search.trim().toLowerCase()));
  const groups = [...new Set(shown.map((d) => d.group))];
  const toggle = (d: FacetDef<T>, value: string, all: string[]) => {
    const cur = sel[d.key];
    let next: Set<string>;
    if (!cur || !cur.size) next = new Set(all.filter((v) => v !== value));
    else {
      next = new Set(cur);
      if (next.has(value)) next.delete(value);
      else next.add(value);
      if (next.size === all.length) next = new Set();
    }
    onChange({ ...sel, [d.key]: next });
  };
  return (
    <aside className="wpm-facets">
      {top}
      <label className="wpm-facet-search">
        <Search size={13} />
        <input placeholder="Search facets" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {groups.map((g) => (
        <div key={g}>
          <button type="button" className="wpm-facet-group" onClick={() => setHidden((h) => ({ ...h, [g]: !h[g] }))}>
            {hidden[g] ? <ChevronRight size={13} /> : <ChevronDown size={13} />} {g}
          </button>
          {!hidden[g] &&
            shown
              .filter((d) => d.group === g)
              .map((d) => {
                const counts = new Map<string, number>();
                for (const r of rows) for (const v of valuesOf(d, r)) counts.set(v, (counts.get(v) ?? 0) + 1);
                const all = [...counts.keys()].sort();
                const s = sel[d.key];
                const filtered = !!s && s.size > 0;
                return (
                  <div key={d.key} className="wpm-facet">
                    <div className="wpm-facet-head">
                      <button type="button" onClick={() => setOpen((o) => ({ ...o, [d.key]: !o[d.key] }))}>
                        {open[d.key] ? <ChevronDown size={14} /> : <ChevronRight size={14} />} {d.label}
                      </button>
                      {filtered && (
                        <button type="button" className="wpm-facet-clear" title="Show all values" onClick={() => onChange({ ...sel, [d.key]: new Set() })}>
                          <X size={12} />
                        </button>
                      )}
                    </div>
                    {open[d.key] &&
                      (all.length ? (
                        all.map((v) => (
                          <label key={v} className="wpm-facet-value">
                            <input type="checkbox" className="checkbox" checked={!filtered || s!.has(v)} onChange={() => toggle(d, v, all)} />
                            <span className="wpm-facet-label" title={v}>
                              {v}
                            </span>
                            <span className="muted">{counts.get(v)}</span>
                          </label>
                        ))
                      ) : (
                        <p className="wpm-facet-none">No matching values found</p>
                      ))}
                  </div>
                );
              })}
        </div>
      ))}
    </aside>
  );
}

/** "Hide Controls | N things found" with the actions on the right. */
export function Toolbar({ controls, onToggle, children, right }: { controls: boolean; onToggle: () => void; children: ReactNode; right?: ReactNode }) {
  return (
    <div className="wpm-toolbar">
      <button type="button" className="link-btn wpm-hide" onClick={onToggle}>
        {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
      </button>
      <span className="wpm-sep" />
      {children}
      <span className="syn-grow" />
      {right}
    </div>
  );
}

export function downloadCsv(name: string, header: string[], rows: (string | number)[][]) {
  const cell = (v: string | number) => (/[",\n]/.test(String(v)) ? `"${String(v).replace(/"/g, '""')}"` : String(v));
  const blob = new Blob([[header, ...rows].map((r) => r.map(cell).join(',')).join('\n')], { type: 'text/csv' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

// ---------------------------------------------------------------- agents

const PAGE = 25;
const yes = (b: boolean) => (b ? 'true' : 'false');
const semver = (v: string) => v.split(/[.-]/).map((x) => parseInt(x, 10) || 0);
const cmp = (a: string, b: string) => {
  const x = semver(a);
  const y = semver(b);
  for (let i = 0; i < 3; i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) - (y[i] ?? 0);
  return 0;
};
/** An Agent three or more minor versions behind the newest one reporting. */
const outdated = (v: string, latest: string) => {
  const [a, b] = [semver(v), semver(latest)];
  return a[0] < b[0] || b[1] - a[1] >= 3;
};
const fullyMonitored = (a: CsmAgent) => a.isCwsEnabled && a.isCspmEnabled && a.isCsmVmHostsEnabled && (!a.clusterName.length || a.isCsmVmContainersEnabled);

function coverage(n: number, total: number): { label: string; tone: string } {
  const p = total ? n / total : 0;
  if (p < 0.25) return { label: 'Very low coverage', tone: 'bad' };
  if (p < 0.5) return { label: 'Low coverage', tone: 'warn' };
  if (p < 0.85) return { label: 'Medium coverage', tone: 'mid' };
  return { label: 'High coverage', tone: 'good' };
}

function CoverageHeader({ icon, title, covered, total, unit }: { icon: ReactNode; title: string; covered: number; total: number; unit: string }) {
  const c = coverage(covered, total);
  return (
    <div className="wpm-cov-head">
      <span className="wpm-cov-icon">{icon}</span>
      <div>
        <h3>{title}</h3>
        <p>
          <span className={`wpm-cov-${c.tone}`}>{c.label}</span>
          <span className="wpm-sep" />
          <span className="muted">
            {covered} of {total} {unit}
          </span>
        </p>
      </div>
    </div>
  );
}

const FEATURES: { key: keyof CsmAgent; label: string; icon: ReactNode; main?: boolean }[] = [
  { key: 'isCwsEnabled', label: 'Workload Protection', icon: <ShieldCheck size={13} />, main: true },
  { key: 'isCspmEnabled', label: 'Misconfigurations', icon: <ClipboardCheck size={13} /> },
  { key: 'isCsmVmHostsEnabled', label: 'Host vulnerabilities', icon: <Bug size={13} /> },
  { key: 'isCsmVmContainersEnabled', label: 'Container vulnerabilities', icon: <Layers size={13} /> },
];

function Features({ a, only }: { a: CsmAgent; only?: boolean }) {
  return (
    <span className="wpm-features">
      {FEATURES.filter((f) => !only || f.main).map((f) => {
        const on = !!a[f.key];
        const tone = on ? 'on' : f.main ? 'warn' : 'off';
        return (
          <span key={f.key} className={`wpm-feature ${tone}`} title={`${f.label}: ${on ? 'enabled' : 'not enabled'}`}>
            {f.icon}
          </span>
        );
      })}
    </span>
  );
}

function AgentVersion({ v, latest }: { v: string; latest: string }) {
  const old = outdated(v, latest);
  return (
    <span className={`wpm-version${old ? ' old' : ''}`} title={old ? `Outdated: the newest Agent reporting is ${latest}` : undefined}>
      <Wrench size={12} /> {v}
    </span>
  );
}

function OsLogo({ a }: { a: CsmAgent }) {
  return hasBrandLogo(a.osLogo) ? (
    <BrandLogo name={a.osLogo} size={18} title={a.os} />
  ) : (
    <span className="wpm-os-fallback" title={a.os}>
      <Server size={15} />
    </span>
  );
}

const HOST_FACETS: FacetDef<CsmAgent>[] = [
  { key: 'host', label: 'Hostname', group: 'HOST', value: (a) => a.hostname },
  { key: 'env', label: 'Environment', group: 'HOST', value: (a) => a.envs },
  { key: 'cluster', label: 'Cluster Name', group: 'HOST', value: (a) => a.clusterName },
  { key: 'os', label: 'Operating System', group: 'HOST', value: (a) => a.os },
  { key: 'cws', label: 'Workload Protection Enabled', group: 'AGENT', value: (a) => yes(a.isCwsEnabled) },
  { key: 'cspm', label: 'Misconfigurations Enabled', group: 'AGENT', value: (a) => yes(a.isCspmEnabled) },
  { key: 'vmh', label: 'Hosts VM Enabled', group: 'AGENT', value: (a) => yes(a.isCsmVmHostsEnabled) },
  { key: 'vmc', label: 'Containers VM Enabled', group: 'AGENT', value: (a) => yes(a.isCsmVmContainersEnabled) },
  { key: 'version', label: 'Agent Version', group: 'AGENT', value: (a) => a.agentVersion },
  { key: 'tool', label: 'Install Method', group: 'AGENT', value: (a) => a.installMethodTool },
  { key: 'toolv', label: 'Install Method Version', group: 'AGENT', value: (a) => a.installMethodInstallerVersion },
];
const SERVERLESS_FACETS: FacetDef<CsmAgent>[] = [
  { key: 'arn', label: 'AWS Fargate ECS Task ARN', group: 'SERVERLESS', value: (a) => a.ecsFargateTaskArn },
  { key: 'fargate', label: 'AWS Fargate Type', group: 'SERVERLESS', value: (a) => a.awsFargate },
  { key: 'host', label: 'Hostname', group: 'HOST', value: (a) => a.hostname },
  { key: 'env', label: 'Environment', group: 'HOST', value: (a) => a.envs },
  { key: 'cluster', label: 'Cluster Name', group: 'HOST', value: (a) => a.clusterName },
  { key: 'version', label: 'Agent Version', group: 'AGENT', value: (a) => a.agentVersion },
  { key: 'tool', label: 'Install Method', group: 'AGENT', value: (a) => a.installMethodTool },
  { key: 'toolv', label: 'Install Method Version', group: 'AGENT', value: (a) => a.installMethodInstallerVersion },
  { key: 'cws', label: 'Workload Protection Status', group: 'AGENT', value: (a) => (a.isCwsEnabled ? 'enabled' : 'not enabled') },
];

const GROUPS: { value: string; label: string; of: (a: CsmAgent) => string }[] = [
  { value: '', label: 'Select value', of: () => '' },
  { value: 'env', label: 'Environment', of: (a) => a.envs.join(', ') || 'none' },
  { value: 'cluster', label: 'Cluster Name', of: (a) => a.clusterName.join(', ') || 'no cluster' },
  { value: 'os', label: 'Operating System', of: (a) => a.os },
  { value: 'version', label: 'Agent Version', of: (a) => a.agentVersion },
];

/** Search box, "Group by" picker and facets over a list of Agents. */
function useAgentList(rows: CsmAgent[], defs: FacetDef<CsmAgent>[]) {
  const [search, setSearch] = useState('');
  const [sel, setSel] = useState<FacetSel>({});
  const [group, setGroup] = useState('');
  const [offset, setOffset] = useState(0);
  const filtered = useMemo(() => {
    const t = search.trim().toLowerCase();
    return applyFacets(rows, defs, sel).filter((a) => !t || [a.hostname, a.os, a.agentVersion, a.ecsFargateTaskArn, ...a.envs, ...a.clusterName, ...a.tags].join(' ').toLowerCase().includes(t));
  }, [rows, defs, sel, search]);
  return { search, setSearch: (s: string) => (setSearch(s), setOffset(0)), sel, setSel: (s: FacetSel) => (setSel(s), setOffset(0)), group, setGroup, offset, setOffset, filtered };
}

function SearchGroup({ search, setSearch, group, setGroup }: { search: string; setSearch: (s: string) => void; group: string; setGroup: (g: string) => void }) {
  return (
    <div className="wpm-searchbar">
      <span className="wpm-addon">Search for</span>
      <label>
        <Search size={14} />
        <input placeholder="Enter your query" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      <span className="wpm-addon">Group by</span>
      <select value={group} onChange={(e) => setGroup(e.target.value)} aria-label="Group by">
        {GROUPS.map((g) => (
          <option key={g.value} value={g.value}>
            {g.label}
          </option>
        ))}
      </select>
    </div>
  );
}

/** Rows, optionally under group header rows, a page at a time. */
function grouped(rows: CsmAgent[], group: string, offset: number) {
  const page = rows.slice(offset, offset + PAGE);
  const of = GROUPS.find((g) => g.value === group)?.of;
  if (!group || !of) return [{ name: '', rows: page }];
  const m = new Map<string, CsmAgent[]>();
  for (const a of page) m.set(of(a), [...(m.get(of(a)) ?? []), a]);
  return [...m.entries()].sort().map(([name, rows]) => ({ name, rows }));
}

function AddAgentsModal({ onClose, serverless }: { onClose: () => void; serverless?: boolean }) {
  const helm = `datadog:
  apiKeyExistingSecret: datadog-secret
  site: <your ObserveCraft intake host>
  securityAgent:
    runtime:
      enabled: true
  # Point the runtime security events at the intake
  env:
    - name: DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_DD_URL
      value: https://<intake>:4318`;
  const fargate = `"containerDefinitions": [
  {
    "name": "cws-instrumentation-init",
    "image": "datadog/cws-instrumentation:latest",
    "command": ["/cws-instrumentation", "setup", "--cws-volume-mount", "/cws-instrumentation-volume"]
  },
  {
    "name": "datadog-agent",
    "image": "public.ecr.aws/datadog/agent:latest",
    "environment": [
      { "name": "DD_RUNTIME_SECURITY_CONFIG_ENABLED", "value": "true" },
      { "name": "DD_RUNTIME_SECURITY_CONFIG_EBPFLESS_ENABLED", "value": "true" },
      { "name": "DD_RUNTIME_SECURITY_CONFIG_ENDPOINTS_LOGS_DD_URL", "value": "https://<intake>:4318" }
    ]
  }
]`;
  return (
    <Modal onClose={onClose} label="Add Agents" width={760} height="auto">
      <div className="sec-modal wpm-modal">
        <header>
          <h3>{serverless ? 'Enable Workload Protection on Fargate tasks' : 'Enable Workload Protection on your Agents'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <p className="muted">
          Workload Protection runs in the Datadog Agent&rsquo;s security agent and system probe. Turn runtime security on and send its events to your ObserveCraft intake; the complete Helm values and
          Datadog Operator manifest are in <code>deploy/examples/datadog</code>.
        </p>
        <h4>{serverless ? 'ECS task definition' : 'Helm (values.yaml)'}</h4>
        <pre className="wpm-code">{serverless ? fargate : helm}</pre>
        {!serverless && (
          <>
            <h4>Datadog Operator (DatadogAgent)</h4>
            <pre className="wpm-code">{`spec:
  features:
    cws:
      enabled: true`}</pre>
          </>
        )}
        <p className="muted">
          Agents load the default policies they ship with. To run your own policies, download them from Policies → Agent configuration and mount them in{' '}
          <code>/etc/datadog-agent/runtime-security.d</code>.
        </p>
        <footer>
          <button type="button" className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </footer>
      </div>
    </Modal>
  );
}

function AgentsTable({ serverless }: { serverless?: boolean }) {
  const q = useWpAgents();
  const all = (serverless ? q.data?.serverless : q.data?.hosts) ?? [];
  const defs = serverless ? SERVERLESS_FACETS : HOST_FACETS;
  const list = useAgentList(all, defs);
  const [controls, setControls] = useState(true);
  const [adding, setAdding] = useState(false);
  const latest =
    all
      .map((a) => a.agentVersion)
      .sort(cmp)
      .at(-1) ?? '';
  const covered = all.filter((a) => (serverless ? a.isCwsEnabled : fullyMonitored(a))).length;
  const csv = () =>
    downloadCsv(
      serverless ? 'workload-protection-serverless.csv' : 'workload-protection-agents.csv',
      ['hostname', 'os', 'agent_version', 'cluster', 'env', 'task_arn', 'workload_protection', 'misconfigurations', 'host_vulnerabilities', 'container_vulnerabilities'],
      list.filtered.map((a) => [
        a.hostname,
        a.os,
        a.agentVersion,
        a.clusterName.join(' '),
        a.envs.join(' '),
        a.ecsFargateTaskArn,
        yes(a.isCwsEnabled),
        yes(a.isCspmEnabled),
        yes(a.isCsmVmHostsEnabled),
        yes(a.isCsmVmContainersEnabled),
      ]),
    );
  const cols = serverless ? 5 : 5;
  return (
    <div className="sec-page wpm-page">
      <WpTabs tabs={INVENTORY_TABS} />
      {serverless ? (
        <CoverageHeader icon={<i className="wpm-fx">fx</i>} title="Serverless Resources" covered={covered} total={all.length} unit="tasks covered for Workload Protection" />
      ) : (
        <CoverageHeader icon={<Server size={22} />} title="Hosts and Containers" covered={covered} total={all.length} unit="fully monitored hosts" />
      )}
      <SearchGroup search={list.search} setSearch={list.setSearch} group={list.group} setGroup={list.setGroup} />
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className={`wpm-body${controls ? '' : ' no-facets'}`}>
          {controls && <FacetPanel rows={all} defs={defs} sel={list.sel} onChange={list.setSel} />}
          <div className="wpm-main">
            <Toolbar
              controls={controls}
              onToggle={() => setControls((c) => !c)}
              right={
                <>
                  {!serverless && (
                    <button className="btn" onClick={() => setAdding(true)}>
                      Configure Workload Protection
                    </button>
                  )}
                  <button className="btn" onClick={() => setAdding(true)}>
                    <Plus size={14} /> Add Agents
                  </button>
                  {!serverless && (
                    <button className="btn wpm-iconbtn" onClick={csv} title="Download as CSV" aria-label="Download as CSV">
                      <Download size={14} />
                    </button>
                  )}
                  <button className="btn wpm-iconbtn" title="Columns" aria-label="Columns" disabled>
                    <Settings2 size={14} />
                  </button>
                  <button className="btn wpm-iconbtn" onClick={() => q.refetch()} title="Refresh" aria-label="Refresh">
                    <RefreshCw size={14} />
                  </button>
                </>
              }
            >
              <span>
                <b>{list.filtered.length}</b> agents found
              </span>
            </Toolbar>
            {!list.filtered.length ? (
              <Empty>
                <b>No agents found.</b>
                <p className="muted">{all.length ? 'No Agent matches these filters.' : 'Agents appear here once they send their host metadata to the intake.'}</p>
              </Empty>
            ) : (
              <table className="mon-table wpm-table">
                <thead>
                  {serverless ? (
                    <tr>
                      <th>AWS FARGATE TYPE</th>
                      <th>AWS FARGATE ECS TASK ARN</th>
                      <th>AGENT VERSION</th>
                      <th>CLUSTER NAME</th>
                      <th className="wpm-right">FEATURES</th>
                    </tr>
                  ) : (
                    <tr>
                      <th className="wpm-os">OS</th>
                      <th>HOSTNAME</th>
                      <th>AGENT VERSION</th>
                      <th>CLUSTER NAME</th>
                      <th className="wpm-right">FEATURES</th>
                    </tr>
                  )}
                </thead>
                <tbody>
                  {grouped(list.filtered, list.group, list.offset).map((g) => (
                    <Fragment key={g.name || 'all'}>
                      {g.name && (
                        <tr className="wpm-group-row">
                          <td colSpan={cols}>
                            {g.name} <span className="muted">({g.rows.length})</span>
                          </td>
                        </tr>
                      )}
                      {g.rows.map((a) =>
                        serverless ? (
                          <tr key={a.id}>
                            <td className="nowrap">
                              <span className="wpm-fargate">
                                {hasBrandLogo(a.platformLogo || 'amazonecs') && <BrandLogo name={a.platformLogo || 'amazonecs'} size={18} />} Amazon {a.awsFargate || 'ECS'}
                              </span>
                            </td>
                            <td>
                              <span className="wpm-chip mono" title={a.ecsFargateTaskArn}>
                                {a.ecsFargateTaskArn.split('/').at(-1) || a.hostname}
                              </span>
                            </td>
                            <td>
                              <AgentVersion v={a.agentVersion} latest={latest} />
                            </td>
                            <td>
                              {a.clusterName.map((c) => (
                                <span key={c} className="wpm-chip">
                                  {c}
                                </span>
                              ))}
                            </td>
                            <td className="wpm-right">
                              <Features a={a} only />
                            </td>
                          </tr>
                        ) : (
                          <tr key={a.id}>
                            <td className="wpm-os">
                              <OsLogo a={a} />
                            </td>
                            <td>
                              <span className="wpm-chip" title={`Last seen ${timeAgo(a.lastSeen)}`}>
                                {a.hostname}
                              </span>
                            </td>
                            <td>
                              <AgentVersion v={a.agentVersion} latest={latest} />
                            </td>
                            <td>
                              {a.clusterName.map((c) => (
                                <span key={c} className="wpm-chip">
                                  {c}
                                </span>
                              ))}
                            </td>
                            <td className="wpm-right">
                              <Features a={a} />
                            </td>
                          </tr>
                        ),
                      )}
                    </Fragment>
                  ))}
                </tbody>
              </table>
            )}
            <div className="wpm-pager">
              <Pager offset={list.offset} limit={PAGE} total={list.filtered.length} onChange={list.setOffset} />
            </div>
          </div>
        </div>
      )}
      {adding && <AddAgentsModal serverless={serverless} onClose={() => setAdding(false)} />}
    </div>
  );
}

/** Inventory → Hosts & Containers: every Agent, its version and which security features run on it. */
export function HostsContainersPage() {
  return <AgentsTable />;
}

/** Inventory → Serverless: the Agents running as sidecars of AWS Fargate tasks. */
export function ServerlessPage() {
  return <AgentsTable serverless />;
}

// ---------------------------------------------------------------- coverage

function Bar({ n, total }: { n: number; total: number }) {
  const p = total ? Math.round((n / total) * 100) : 0;
  return (
    <span className="wpm-bar-cell">
      <span className="wpm-bar">
        <i style={{ width: `${p}%` }} />
      </span>
      <b>{p}%</b>
      <span className="muted">
        {n} of {total}
      </span>
    </span>
  );
}

/** Inventory → Coverage: how much of the infrastructure each security feature covers, per environment. */
export function CoveragePage() {
  const q = useWpAgents();
  const hosts = q.data?.hosts ?? [];
  const tasks = q.data?.serverless ?? [];
  const latest =
    [...hosts, ...tasks]
      .map((a) => a.agentVersion)
      .sort(cmp)
      .at(-1) ?? '';
  const envs = [...new Set(hosts.flatMap((a) => (a.envs.length ? a.envs : ['none'])))].sort();
  const card = (title: string, n: number, total: number, unit: string) => {
    const c = coverage(n, total);
    return (
      <div className="csm-card wpm-card">
        <h5>{title}</h5>
        <p>
          <b>{n}</b> of <b>{total}</b> {unit}
        </p>
        <div className="wpm-bar">
          <i style={{ width: `${total ? (n / total) * 100 : 0}%` }} />
        </div>
        <span className={`wpm-cov-${c.tone}`}>{c.label}</span>
      </div>
    );
  };
  return (
    <div className="sec-page wpm-page">
      <WpTabs tabs={INVENTORY_TABS} />
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="wpm-cards">
            {card('Hosts and containers', hosts.filter((a) => a.isCwsEnabled).length, hosts.length, 'hosts with Workload Protection')}
            {card('Serverless resources', tasks.filter((a) => a.isCwsEnabled).length, tasks.length, 'tasks with Workload Protection')}
            {card('Fully monitored hosts', hosts.filter(fullyMonitored).length, hosts.length, 'hosts with every security feature')}
            {card('Up-to-date Agents', [...hosts, ...tasks].filter((a) => !outdated(a.agentVersion, latest)).length, hosts.length + tasks.length, 'Agents')}
          </div>
          <h3 className="wpm-h3">Coverage by environment</h3>
          {!envs.length ? (
            <Empty>
              <b>No Agents yet.</b>
            </Empty>
          ) : (
            <table className="mon-table wpm-table">
              <thead>
                <tr>
                  <th>ENVIRONMENT</th>
                  <th className="num">HOSTS</th>
                  <th>WORKLOAD PROTECTION</th>
                  <th>MISCONFIGURATIONS</th>
                  <th>HOST VULNERABILITIES</th>
                  <th>CONTAINER VULNERABILITIES</th>
                </tr>
              </thead>
              <tbody>
                {envs.map((e) => {
                  const rows = hosts.filter((a) => (a.envs.length ? a.envs.includes(e) : e === 'none'));
                  const withContainers = rows.filter((a) => a.clusterName.length);
                  return (
                    <tr key={e}>
                      <td>
                        <span className="wpm-chip">env:{e}</span>
                      </td>
                      <td className="num">{rows.length}</td>
                      <td>
                        <Bar n={rows.filter((a) => a.isCwsEnabled).length} total={rows.length} />
                      </td>
                      <td>
                        <Bar n={rows.filter((a) => a.isCspmEnabled).length} total={rows.length} />
                      </td>
                      <td>
                        <Bar n={rows.filter((a) => a.isCsmVmHostsEnabled).length} total={rows.length} />
                      </td>
                      <td>{withContainers.length ? <Bar n={withContainers.filter((a) => a.isCsmVmContainersEnabled).length} total={withContainers.length} /> : <span className="muted">—</span>}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          <h3 className="wpm-h3">Agents to update</h3>
          {(() => {
            const old = [...hosts, ...tasks].filter((a) => outdated(a.agentVersion, latest));
            return !old.length ? (
              <p className="muted">Every Agent runs a recent version.</p>
            ) : (
              <table className="mon-table wpm-table">
                <thead>
                  <tr>
                    <th>HOSTNAME</th>
                    <th>AGENT VERSION</th>
                    <th>INSTALL METHOD</th>
                    <th>LAST SEEN</th>
                  </tr>
                </thead>
                <tbody>
                  {old.map((a) => (
                    <tr key={a.id}>
                      <td>
                        <span className="wpm-chip">{a.hostname}</span>
                      </td>
                      <td>
                        <AgentVersion v={a.agentVersion} latest={latest} />
                      </td>
                      <td>{a.installMethodTool}</td>
                      <td className="muted">{timeAgo(a.lastSeen)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            );
          })()}
        </>
      )}
    </div>
  );
}

import { useMemo, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import { BellOff, Bell, ExternalLink, PanelLeftClose, PanelLeftOpen, Search, X } from 'lucide-react';
import type { FindingStatus, InfraVulnerability, MisconfigFinding } from '../../../api/cloudSecurity/client';
import type { Facet } from '../../../api/types';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../../lib/searchQuery';
import { SeverityBadge } from '../common';
import { FINDING_TABS, SubTabs } from './CloudSecurityLayout';
import { useCve, useInfraVulnerabilities, useMisconfigurations, useTriage } from './queries';

const RANK = ['low', 'medium', 'high', 'critical'];

/** Facets of rows: per key, the values and their counts. */
function facetsOf<T>(rows: T[], keys: [string, string, string, (r: T) => (string | null | undefined)[]][]): Facet[] {
  return keys.map(([key, label, group, get]) => {
    const m = new Map<string, number>();
    for (const r of rows) for (const v of get(r)) if (v) m.set(v, (m.get(v) ?? 0) + 1);
    return { key, label, group, values: [...m.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value)) };
  });
}

/** Whether a row matches the search terms (tag terms by facet key, free text anywhere). */
function matches<T>(r: T, terms: Term[], get: (r: T, key: string) => (string | null | undefined)[], text: (r: T) => string): boolean {
  return terms.every((t) => {
    const hit =
      t.kind === 'text'
        ? text(r).toLowerCase().includes(t.text.toLowerCase())
        : t.values.some((p) => {
            const re = new RegExp(
              `^${p
                .toLowerCase()
                .split('*')
                .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
                .join('.*')}$`,
            );
            return get(r, t.key).some((v) => !!v && re.test(v.toLowerCase()));
          });
    return hit !== t.negate;
  });
}

function useExplorerParams() {
  const [params, setParams] = useSearchParams();
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  return { params, set };
}

function Toolbar({
  query,
  onQuery,
  groups,
  group,
  onGroup,
  status,
  onStatus,
  count,
  noun,
  controls,
  onControls,
  extra,
}: {
  query: string;
  onQuery: (q: string) => void;
  groups: [string, string][];
  group: string;
  onGroup: (g: string) => void;
  status: FindingStatus;
  onStatus: (s: FindingStatus) => void;
  count: number;
  noun: string;
  controls: boolean;
  onControls: () => void;
  extra?: ReactNode;
}) {
  const [draft, setDraft] = useState(query);
  return (
    <>
      <form
        className="sec-search csm-search"
        onSubmit={(e) => {
          e.preventDefault();
          onQuery(draft.trim());
        }}
      >
        <Search size={15} />
        <input value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => onQuery(draft.trim())} placeholder="Search: severity:high namespace:payments @rule:ocs-k8s-001 …" />
      </form>
      <div className="csm-groupby">
        <span>Group by</span>
        {groups.map(([v, l]) => (
          <button key={v} className={group === v ? 'active' : ''} onClick={() => onGroup(v)}>
            {l}
          </button>
        ))}
        {extra}
        <span className="syn-grow" />
        <div className="sec-seg">
          <span>Status</span>
          {(['open', 'muted', 'all'] as const).map((s) => (
            <button key={s} className={status === s ? 'active' : ''} onClick={() => onStatus(s)}>
              {s[0].toUpperCase() + s.slice(1)}
            </button>
          ))}
        </div>
      </div>
      <div className="syn-toolbar">
        <button className="link-btn" onClick={onControls}>
          {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
        </button>
        <span className="syn-toolbar-sep" />
        <span>
          <b>{compact(count)}</b> {noun}
        </span>
      </div>
    </>
  );
}

function MuteButton({ k, status }: { k: string; status: string }) {
  const triage = useTriage();
  return (
    <button className="btn btn-sm" disabled={triage.isPending} onClick={(e) => (e.stopPropagation(), triage.mutate({ key: k, status: status === 'muted' ? 'open' : 'muted' }))}>
      {status === 'muted' ? <Bell size={13} /> : <BellOff size={13} />} {status === 'muted' ? 'Unmute' : 'Mute'}
    </button>
  );
}

// ================================================================ misconfigurations

const MISCONFIG_FIELDS = (f: MisconfigFinding, key: string): (string | null | undefined)[] => {
  switch (key.replace(/^@/, '')) {
    case 'severity':
    case 'status':
      return key.endsWith('status') ? [f.status] : [f.severity];
    case 'rule':
      return [f.ruleId, f.ruleName];
    case 'resource_type':
      return [f.resourceType];
    case 'cluster':
      return [f.cluster];
    case 'namespace':
      return [f.namespace];
    case 'env':
      return [f.env];
    case 'service':
      return [f.service];
    case 'team':
      return [f.team];
    case 'account':
      return [f.account];
    case 'framework':
      return f.frameworks;
    default:
      return [];
  }
};

/** Cloud Security → Findings → Misconfigurations. */
export function MisconfigurationsPage() {
  const { params, set } = useExplorerParams();
  const status = (params.get('status') ?? 'open') as FindingStatus;
  const group = params.get('group') ?? 'finding';
  const query = params.get('q') ?? (params.get('rule') ? `@rule:${params.get('rule')}` : '');
  const open = params.get('finding');
  const [controls, setControls] = useState(true);
  const q = useMisconfigurations(status);
  const terms = parseQuery(query);
  const all = q.data?.findings ?? [];
  const rows = all.filter((f) => matches(f, terms, MISCONFIG_FIELDS, (x) => `${x.ruleName} ${x.resourceName} ${x.detail}`));
  const facets = facetsOf(rows, [
    ['severity', 'Severity', 'CORE', (f) => [f.severity]],
    ['@rule', 'Rule', 'CORE', (f) => [f.ruleId]],
    ['resource_type', 'Resource Type', 'CORE', (f) => [f.resourceType]],
    ['cluster', 'Cluster', 'KUBERNETES', (f) => [f.cluster]],
    ['namespace', 'Namespace', 'KUBERNETES', (f) => [f.namespace]],
    ['env', 'Env', 'TAGS', (f) => [f.env]],
    ['service', 'Service', 'TAGS', (f) => [f.service]],
    ['team', 'Team', 'TAGS', (f) => [f.team]],
    ['framework', 'Framework', 'COMPLIANCE', (f) => f.frameworks],
  ]);
  const groups = useMemo(() => {
    const m = new Map<string, MisconfigFinding[]>();
    for (const f of rows) {
      const k = group === 'resource' ? f.resourceId : group === 'none' ? f.key : f.ruleId;
      m.set(k, [...(m.get(k) ?? []), f]);
    }
    return [...m.entries()].sort((a, b) => RANK.indexOf(b[1][0].severity) - RANK.indexOf(a[1][0].severity) || b[1].length - a[1].length);
  }, [rows, group]);
  const rule = q.data?.rules.find((r) => r.id === open) ?? null;
  return (
    <div className="sec-explorer">
      <SubTabs tabs={FINDING_TABS} />
      <Toolbar
        query={query}
        onQuery={(v) => (set('rule', null), set('q', v || null))}
        groups={[
          ['finding', 'Finding'],
          ['resource', 'Resource'],
          ['none', 'None'],
        ]}
        group={group}
        onGroup={(g) => set('group', g === 'finding' ? null : g)}
        status={status}
        onStatus={(s) => set('status', s === 'open' ? null : s)}
        count={groups.length}
        noun={group === 'none' ? 'findings' : 'groups found'}
        controls={controls}
        onControls={() => setControls((c) => !c)}
      />
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={(t: Term[]) => (set('rule', null), set('q', formatQuery(t) || null))}
            defaultOpen={['severity', '@rule', 'resource_type', 'namespace']}
            verified={[]}
            noun="facets"
          />
        )}
        <div className="syn-explorer-main">
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : !groups.length ? (
            <Empty>
              <b>No matching misconfigurations.</b>
              <p className="muted">
                {q.data!.rules.some((r) => r.evaluated)
                  ? 'Every evaluated resource passes these rules.'
                  : 'No resources to evaluate yet: send the Kubernetes objects with the Agent’s orchestrator explorer.'}
              </p>
            </Empty>
          ) : (
            <table className="mon-table syn-table csm-table">
              <thead>
                <tr>
                  <th>{group === 'resource' ? 'RESOURCE' : 'FINDING'}</th>
                  <th>SEVERITY</th>
                  <th className="num">{group === 'resource' ? 'FINDINGS' : 'RESOURCES'}</th>
                  <th>RESOURCE TYPE</th>
                  <th>{group === 'none' ? 'RESOURCE' : 'DETAIL'}</th>
                  <th>STATUS</th>
                </tr>
              </thead>
              <tbody>
                {groups.map(([k, items]) => {
                  const f = items[0];
                  const worst = items.map((x) => x.severity).sort((a, b) => RANK.indexOf(b) - RANK.indexOf(a))[0];
                  return (
                    <tr key={k} className={open === f.ruleId ? 'selected' : ''} onClick={() => set('finding', f.ruleId)}>
                      <td>
                        <b>{group === 'resource' ? f.resourceName : f.ruleName}</b>
                        <div className="muted sec-sub">{group === 'resource' ? [f.cluster, f.namespace].filter(Boolean).join(' / ') : f.ruleId}</div>
                      </td>
                      <td>
                        <SeverityBadge severity={worst} />
                      </td>
                      <td className="num">{items.length}</td>
                      <td>{[...new Set(items.map((x) => x.resourceType.replace(/^kube_/, '')))].join(', ')}</td>
                      <td className="csm-detail">{group === 'none' ? f.resourceName : group === 'resource' ? items.map((x) => x.ruleName).join('; ') : f.detail}</td>
                      <td>{items.every((x) => x.status === 'muted') ? 'Muted' : 'Open'}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {rule &&
        createPortal(
          <Drawer onClose={() => set('finding', null)} title={rule.name} severity={rule.severity} sub={`${rule.id} · ${rule.failing} of ${rule.evaluated} resources fail`}>
            <h4>Description</h4>
            <p>{rule.description}</p>
            <h4>Remediation</h4>
            <p className="csm-remediation">{rule.remediation}</p>
            <div className="sec-tags">
              {rule.frameworks.map((x) => (
                <span key={x} className="tag-chip">
                  {x}
                </span>
              ))}
            </div>
            <h4>Failing resources</h4>
            <table className="mon-table syn-table">
              <tbody>
                {all
                  .filter((x) => x.ruleId === rule.id)
                  .map((x) => (
                    <tr key={x.key}>
                      <td>
                        <b>{x.resourceName}</b>
                        <div className="muted sec-sub">{[x.resourceType.replace(/^kube_/, ''), x.cluster, x.namespace].filter(Boolean).join(' · ')}</div>
                      </td>
                      <td className="csm-detail">{x.detail}</td>
                      <td className="num">
                        <MuteButton k={x.key} status={x.status} />
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </Drawer>,
          document.body,
        )}
    </div>
  );
}

function Drawer({ onClose, title, severity, sub, children }: { onClose: () => void; title: string; severity: string; sub: string; children: ReactNode }) {
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel csm-drawer" role="dialog" aria-label={title}>
        <header className="sec-panel-head">
          <SeverityBadge severity={severity} />
          <h2>{title}</h2>
          <span className="syn-grow" />
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <p className="muted csm-drawer-sub">{sub}</p>
        <div className="csm-drawer-body">{children}</div>
      </aside>
    </div>
  );
}

// ================================================================ vulnerabilities

const VULN_FIELDS = (v: InfraVulnerability, key: string): (string | null | undefined)[] => {
  switch (key.replace(/^@/, '')) {
    case 'severity':
      return [v.severity];
    case 'status':
      return [v.status];
    case 'cve':
      return [v.cve, v.advisoryId];
    case 'package':
      return [v.package];
    case 'ecosystem':
      return [v.ecosystem];
    case 'resource_type':
      return [v.resourceKind];
    case 'resource':
      return [v.resourceName];
    case 'host':
      return [v.host];
    case 'os':
      return [v.os];
    case 'env':
      return [v.env];
    case 'running':
      return [String(v.running)];
    case 'exploit_available':
      return [String(v.exploitAvailable)];
    case 'fix_available':
      return [String(!!v.fixed)];
    default:
      return [];
  }
};

/** Cloud Security → Findings → Vulnerabilities (hosts and container images). */
export function InfraVulnerabilitiesPage() {
  const { params, set } = useExplorerParams();
  const status = (params.get('status') ?? 'open') as FindingStatus;
  const group = params.get('group') ?? 'vulnerability';
  const query = params.get('q') ?? '';
  const open = params.get('vuln');
  const quick = { running: params.get('running') === '1', exploit: params.get('exploit') === '1', fix: params.get('fix') === '1', severe: params.get('severity') === 'high' };
  const [controls, setControls] = useState(true);
  const q = useInfraVulnerabilities(status);
  const terms = parseQuery(query);
  const all = q.data ?? [];
  const rows = all.filter(
    (v) =>
      matches(v, terms, VULN_FIELDS, (x) => `${x.advisoryId} ${x.cve ?? ''} ${x.title} ${x.package} ${x.resourceName}`) &&
      (!quick.running || v.running) &&
      (!quick.exploit || v.exploitAvailable) &&
      (!quick.fix || !!v.fixed) &&
      (!quick.severe || ['high', 'critical'].includes(v.severity)),
  );
  const facets = facetsOf(rows, [
    ['severity', 'Severity', 'CORE', (v) => [v.severity]],
    ['resource_type', 'Resource Type', 'CORE', (v) => [v.resourceKind]],
    ['@package', 'Package', 'CORE', (v) => [v.package]],
    ['ecosystem', 'Ecosystem', 'CORE', (v) => [v.ecosystem]],
    ['os', 'Operating System', 'CORE', (v) => [v.os]],
    ['host', 'Host', 'INFRASTRUCTURE', (v) => [v.host]],
    ['env', 'Env', 'TAGS', (v) => [v.env]],
  ]);
  const groups = useMemo(() => {
    const m = new Map<string, InfraVulnerability[]>();
    for (const v of rows) {
      const k = group === 'package' ? `${v.ecosystem}:${v.package}@${v.version}` : group === 'resource' ? v.resourceId : group === 'none' ? v.key : v.advisoryId;
      m.set(k, [...(m.get(k) ?? []), v]);
    }
    return [...m.entries()].sort((a, b) => RANK.indexOf(b[1][0].severity) - RANK.indexOf(a[1][0].severity) || (b[1][0].score ?? 0) - (a[1][0].score ?? 0));
  }, [rows, group]);
  const toggle = (k: string, on: boolean) => set(k, on ? null : k === 'severity' ? 'high' : '1');
  return (
    <div className="sec-explorer">
      <SubTabs tabs={FINDING_TABS} />
      <Toolbar
        query={query}
        onQuery={(v) => set('q', v || null)}
        groups={[
          ['vulnerability', 'Unique Vulnerabilities'],
          ['package', 'Packages'],
          ['resource', 'Resources'],
          ['none', 'None'],
        ]}
        group={group}
        onGroup={(g) => set('group', g === 'vulnerability' ? null : g)}
        status={status}
        onStatus={(s) => set('status', s === 'open' ? null : s)}
        count={groups.length}
        noun={group === 'none' ? 'vulnerabilities' : 'groups found'}
        controls={controls}
        onControls={() => setControls((c) => !c)}
        extra={
          <span className="csm-quick">
            {(
              [
                ['running', 'Running', quick.running],
                ['exploit', 'Exploit available', quick.exploit],
                ['fix', 'Fix available', quick.fix],
                ['severity', 'High or critical', quick.severe],
              ] as [string, string, boolean][]
            ).map(([k, l, on]) => (
              <label key={k}>
                <input type="checkbox" checked={on} onChange={() => toggle(k, on)} /> {l}
              </label>
            ))}
          </span>
        }
      />
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel facets={facets} terms={terms} onChange={(t: Term[]) => set('q', formatQuery(t) || null)} defaultOpen={['severity', 'resource_type', 'os']} verified={[]} noun="facets" />
        )}
        <div className="syn-explorer-main">
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : !groups.length ? (
            <Empty>
              <b>No matching vulnerabilities.</b>
              <p className="muted">
                {all.length
                  ? 'Change the search or the filters.'
                  : 'Enable SBOMs on the Datadog Agent (DD_SBOM_ENABLED, DD_SBOM_HOST_ENABLED, DD_SBOM_CONTAINER_IMAGE_ENABLED) to scan hosts and images.'}
              </p>
            </Empty>
          ) : (
            <table className="mon-table syn-table csm-table">
              <thead>
                <tr>
                  <th>{group === 'package' ? 'PACKAGE' : group === 'resource' ? 'RESOURCE' : 'VULNERABILITY'}</th>
                  <th>SEVERITY</th>
                  <th>RISKS</th>
                  <th className="num">{group === 'resource' ? 'VULNERABILITIES' : 'RESOURCES'}</th>
                  <th>{group === 'vulnerability' || group === 'none' ? 'PACKAGE' : 'DETAIL'}</th>
                  <th>FIX</th>
                </tr>
              </thead>
              <tbody>
                {groups.map(([k, items]) => {
                  const v = items[0];
                  const res = new Set(items.map((x) => x.resourceId)).size;
                  return (
                    <tr key={k} className={open === v.advisoryId ? 'selected' : ''} onClick={() => set('vuln', v.advisoryId)}>
                      <td>
                        {group === 'package' ? (
                          <b className="mono">
                            {v.package} {v.version}
                          </b>
                        ) : group === 'resource' ? (
                          <b>{v.resourceName}</b>
                        ) : (
                          <>
                            <b>{v.title}</b>
                            <div className="muted sec-sub mono">{v.cve ?? v.advisoryId}</div>
                          </>
                        )}
                      </td>
                      <td className="nowrap">
                        <SeverityBadge severity={v.severity} /> {v.score != null && <span className="muted">{v.score.toFixed(1)}</span>}
                      </td>
                      <td>
                        <div className="sec-tags">
                          {items.some((x) => x.running) && <span className="tag-chip">Running</span>}
                          {items.some((x) => x.exploitAvailable) && <span className="tag-chip sec-ransom">Exploited</span>}
                        </div>
                      </td>
                      <td className="num">{group === 'resource' ? new Set(items.map((x) => x.advisoryId)).size : res}</td>
                      <td className="mono csm-detail">{group === 'resource' ? `${v.resourceKind.replace('_', ' ')} · ${v.os}` : `${v.package} ${v.version}`}</td>
                      <td className="mono">{v.fixed ?? <span className="muted">—</span>}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {open && createPortal(<VulnDrawer id={open} rows={all.filter((v) => v.advisoryId === open)} onClose={() => set('vuln', null)} />, document.body)}
    </div>
  );
}

function VulnDrawer({ id, rows, onClose }: { id: string; rows: InfraVulnerability[]; onClose: () => void }) {
  const a = useCve(id);
  const first = rows[0];
  return (
    <Drawer
      onClose={onClose}
      title={a.data?.summary ?? first?.title ?? id}
      severity={a.data?.severity ?? first?.severity ?? 'medium'}
      sub={`${[...new Set([first?.cve, id].filter(Boolean))].join(' · ')}${a.data?.score ? ` · CVSS ${a.data.score}` : ''}`}
    >
      <ImpactSummary rows={rows} />
      {a.isLoading ? <Loading /> : a.data && <AdvisoryBody a={a.data} />}
      <h4>Impacted resources</h4>
      {!rows.length ? (
        <p className="muted">No host or image of yours has an affected version.</p>
      ) : (
        <table className="mon-table syn-table">
          <tbody>
            {rows.map((v) => (
              <tr key={v.key}>
                <td>
                  <b>{v.resourceName}</b>
                  <div className="muted sec-sub">
                    {v.resourceKind.replace('_', ' ')} · {v.os} · {v.running ? 'running' : 'not running'} · seen {timeAgo(v.lastSeen)}
                  </div>
                </td>
                <td className="mono">
                  {v.package} {v.version}
                  {v.fixed && <div className="muted sec-sub">fixed in {v.fixed}</div>}
                </td>
                <td className="num">
                  <MuteButton k={v.key} status={v.status} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Drawer>
  );
}

function ImpactSummary({ rows }: { rows: InfraVulnerability[] }) {
  const hosts = new Set(rows.filter((v) => v.resourceKind === 'host').map((v) => v.resourceId)).size;
  const images = new Set(rows.filter((v) => v.resourceKind !== 'host').map((v) => v.resourceId)).size;
  return (
    <div className="csm-impact">
      <div className={hosts ? 'bad' : 'ok'}>
        <b>{hosts}</b> hosts impacted
      </div>
      <div className={images ? 'bad' : 'ok'}>
        <b>{images}</b> container images impacted
      </div>
      <div className={rows.some((v) => v.running) ? 'bad' : 'ok'}>
        <b>{new Set(rows.filter((v) => v.running).map((v) => v.resourceId)).size}</b> running
      </div>
    </div>
  );
}

export function AdvisoryBody({ a }: { a: NonNullable<ReturnType<typeof useCve>['data']> }) {
  return (
    <>
      <h4>Summary</h4>
      <p>{a.details || a.summary}</p>
      <div className="sec-tags">
        {a.cwes.map((c) => (
          <span key={c} className="tag-chip">
            {c}
          </span>
        ))}
        {a.published && <span className="muted">Published {new Date(a.published).toLocaleDateString()}</span>}
        <span className="muted">Source: {a.source}</span>
      </div>
      <h4>Impacted packages</h4>
      <table className="mon-table syn-table">
        <thead>
          <tr>
            <th>PACKAGE NAME</th>
            <th>ECOSYSTEM</th>
            <th>AFFECTED VERSIONS</th>
            <th>FIXED VERSION</th>
          </tr>
        </thead>
        <tbody>
          {a.packages.map((p) => (
            <tr key={`${p.ecosystem}:${p.package}`}>
              <td className="mono">{p.package}</td>
              <td>{p.ecosystem}</td>
              <td className="mono">{p.ranges.map((r) => `${r.introduced && r.introduced !== '0' ? `≥ ${r.introduced} ` : ''}${r.fixed ? `< ${r.fixed}` : ''}`).join('; ')}</td>
              <td className="mono">{p.fixed || '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {a.refs.length > 0 && (
        <>
          <h4>Reference links</h4>
          <ul className="csm-refs">
            {a.refs.slice(0, 8).map((r) => (
              <li key={r.url}>
                <a href={r.url} target="_blank" rel="noreferrer noopener">
                  {r.url} <ExternalLink size={12} />
                </a>
              </li>
            ))}
          </ul>
        </>
      )}
      <p className="muted csm-code-link">
        Libraries of your services: <Link to={`/code-security/vulnerabilities/libraries?q=${a.id}`}>open in Code Security</Link>
      </p>
    </>
  );
}

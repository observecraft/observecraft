import { useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowUpCircle, AlertTriangle, PanelLeft, Plus, ServerCog } from 'lucide-react';
import type { FleetAgent, FleetPage } from '../../api/fleet/types';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { AgentPanel } from './AgentPanel';
import { ago, kindLabel } from './fleetModel';
import { IntegrationLogos, KindCell, OsCell, statusLabel, StatusPill, VersionCell } from './fleetUi';
import { useFleet } from './queries';

const FACET_LABELS: Record<string, string> = {
  type: 'Type',
  status: 'Status',
  version: 'Agent Version',
  os: 'OS',
  integration: 'Integration',
  cluster: 'Cluster',
  env: 'Env',
  install_method: 'Install Method',
};

/** Query string state: `q` (the search) and `agent` (the open side panel). */
function useFleetParams() {
  const [params, setParams] = useSearchParams();
  const set = (patch: Record<string, string | null>) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        for (const [k, v] of Object.entries(patch)) {
          if (v) n.set(k, v);
          else n.delete(k);
        }
        return n;
      },
      { replace: true },
    );
  return { query: params.get('q') ?? '', open: params.get('agent'), set };
}

/** Integrations → Fleet Automation → Fleet View: every agent and collector, its health and version. */
export function FleetViewPage() {
  const { query, open, set } = useFleetParams();
  const page = useFleet(query);
  if (page.isError) return <ErrorState error={page.error} onRetry={() => page.refetch()} />;
  if (!page.data) return <Loading label="Loading agents…" />;
  if (!page.data.enabled) return <FleetEmpty />;
  return (
    <div className="fl-view">
      <h1 className="fl-title">See every Datadog Agent and OpenTelemetry Collector that sends to ObserveCraft, and keep them healthy and current</h1>
      <SummaryCards page={page.data} onQuery={(q) => set({ q })} />
      <AgentExplorer page={page.data} query={query} onQuery={(q) => set({ q: q || null })} selected={open} onOpen={(id) => set({ agent: id })} />
      {open && <AgentPanel id={open} onClose={() => set({ agent: null })} />}
    </div>
  );
}

function FleetEmpty() {
  const navigate = useNavigate();
  return (
    <div className="fl-empty">
      <ServerCog size={72} strokeWidth={1.4} />
      <h2>You haven’t added any Agents yet</h2>
      <p>Install the Datadog Agent or an OpenTelemetry Collector on your hosts and clusters. They appear here as soon as they report, with their version, integrations and health.</p>
      <button className="btn btn-primary" onClick={() => navigate('/infrastructure/add')}>
        <Plus size={14} /> Add Your First Agent
      </button>
    </div>
  );
}

/** Donut of covered hosts. */
function CoverageRing({ pct }: { pct: number | null }) {
  const r = 22;
  const c = 2 * Math.PI * r;
  const filled = ((pct ?? 0) / 100) * c;
  return (
    <svg className="fl-ring" width="56" height="56" viewBox="0 0 56 56" role="img" aria-label={`${pct ?? 0}% of hosts covered`}>
      <circle cx="28" cy="28" r={r} className="fl-ring-track" />
      <circle cx="28" cy="28" r={r} className="fl-ring-fill" strokeDasharray={`${filled} ${c - filled}`} transform="rotate(-90 28 28)" />
    </svg>
  );
}

function SummaryCards({ page, onQuery }: { page: FleetPage; onQuery: (q: string) => void }) {
  const s = page.summary;
  const plural = (n: number) => (n === 1 ? 'Agent is' : 'Agents are');
  const latest = [page.latest.datadogAgent && `Datadog Agent ${page.latest.datadogAgent}`, page.latest.otelCollector && `OTel Collector ${page.latest.otelCollector}`].filter(Boolean).join(' · ');
  return (
    <div className="fl-cards">
      <button className="fl-card" onClick={() => onQuery('outdated:true')} disabled={!s.outdated}>
        <span className="fl-card-tile fl-tile-upgrade">
          <ArrowUpCircle size={22} />
        </span>
        <span className="fl-card-text">
          <strong>
            {s.outdated} {plural(s.outdated)} due for an upgrade
          </strong>
          <span className="muted">{latest ? `Newest in your fleet: ${latest}` : 'No versions reported yet'}</span>
        </span>
      </button>
      <button className="fl-card" onClick={() => onQuery('status:issues')} disabled={!s.withIssues}>
        <span className="fl-card-tile fl-tile-issues">
          <AlertTriangle size={22} />
        </span>
        <span className="fl-card-text">
          <strong>
            {s.withIssues} {plural(s.withIssues)} reporting telemetry issues
          </strong>
          <span className="muted">{s.offline ? `${s.offline} more not reporting at all` : 'Stopped metrics, failing checks or service checks'}</span>
        </span>
      </button>
      <div className="fl-card fl-card-static">
        <CoverageRing pct={s.coverage.pct ?? null} />
        <span className="fl-card-text">
          <strong>{s.coverage.pct == null ? 'No hosts seen yet' : `${s.coverage.pct}% of hosts have an Agent`}</strong>
          <span className="muted">
            {s.coverage.covered} of {s.coverage.hosts} hosts seen in metrics run an Agent or Collector
          </span>
        </span>
      </div>
    </div>
  );
}

type SortKey = 'hostname' | 'version' | 'type' | 'status' | 'lastReported';

function AgentExplorer({ page, query, onQuery, selected, onOpen }: { page: FleetPage; query: string; onQuery: (q: string) => void; selected: string | null; onOpen: (id: string) => void }) {
  const [showFacets, setShowFacets] = useState(true);
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'hostname', desc: false });
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo(() => page.facets.map((f) => ({ key: f.key, label: FACET_LABELS[f.key] ?? f.key, group: '', values: f.values })), [page.facets]);
  const rows = useMemo(() => {
    const value = (a: FleetAgent): string | number => {
      switch (sort.key) {
        case 'version':
          return a.version ?? '';
        case 'type':
          return kindLabel(a.kind);
        case 'status':
          return statusLabel(a.status);
        case 'lastReported':
          return a.lastReported ?? 0;
        default:
          return a.hostname;
      }
    };
    return [...page.agents].sort((a, b) => {
      const [x, y] = [value(a), value(b)];
      const o = typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y), undefined, { numeric: true });
      return sort.desc ? -o : o;
    });
  }, [page.agents, sort]);
  const th = (key: SortKey, label: string) => (
    <th className="fl-sort" onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : key === 'lastReported' }))}>
      {label}
      {sort.key === key ? (sort.desc ? ' ↓' : ' ↑') : ''}
    </th>
  );
  return (
    <>
      <div className="fl-query">
        <QueryBar value={query} onChange={onQuery} placeholder="Filter agents, such as status:issues integration:redisdb cluster:shop-prod-eu" />
      </div>
      <div className="fl-explorer">
        {showFacets && (
          <div className="fl-facets">
            <FacetPanel
              facets={facets}
              terms={terms}
              onChange={(next: Term[]) => onQuery(formatQuery(next))}
              valueSearch
              defaultOpen={['type', 'status', 'version', 'os', 'integration', 'cluster', 'env']}
              verified={[]}
            />
          </div>
        )}
        <div className="fl-results">
          <div className="fl-results-head">
            <button className="icon-btn" onClick={() => setShowFacets((s) => !s)} title={showFacets ? 'Hide facets' : 'Show facets'}>
              <PanelLeft size={16} />
            </button>
            <span>
              Showing {rows.length ? `1–${rows.length}` : 0} of {rows.length} {rows.length === 1 ? 'Agent' : 'Agents'}
            </span>
          </div>
          <div className="fl-table-wrap">
            <table className="fl-table">
              <thead>
                <tr>
                  {th('hostname', 'HOSTNAME')}
                  {th('version', 'AGENT VERSION')}
                  {th('type', 'TYPE')}
                  <th>OS</th>
                  {th('status', 'STATUS')}
                  <th>INTEGRATIONS</th>
                  <th>ENV / CLUSTER</th>
                  {th('lastReported', 'LAST REPORTED')}
                </tr>
              </thead>
              <tbody>
                {rows.map((a) => (
                  <tr key={a.id} className={selected === a.id ? 'selected' : ''} onClick={() => onOpen(a.id)}>
                    <td className="fl-host">{a.hostname}</td>
                    <td>
                      <VersionCell agent={a} />
                    </td>
                    <td>
                      <KindCell kind={a.kind} />
                    </td>
                    <td className="fl-os">
                      <OsCell agent={a} />
                    </td>
                    <td title={a.issues.join('\n') || undefined}>
                      <StatusPill status={a.status} />
                    </td>
                    <td>
                      <IntegrationLogos integrations={a.integrations} />
                    </td>
                    <td>
                      <span className="fl-chips">
                        {a.env && <span className="tag-chip">env:{a.env}</span>}
                        {a.cluster && <span className="tag-chip">{a.cluster}</span>}
                        {!a.env && !a.cluster && <span className="muted">—</span>}
                      </span>
                    </td>
                    <td className="muted" title={a.lastReported ? new Date(a.lastReported).toLocaleString() : undefined}>
                      {ago(a.lastReported)}
                    </td>
                  </tr>
                ))}
                {rows.length === 0 && (
                  <tr>
                    <td colSpan={8} className="fl-table-empty">
                      No agents match this query.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </>
  );
}

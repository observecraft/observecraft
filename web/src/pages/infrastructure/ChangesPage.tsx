import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ExternalLink, PanelLeft, PanelLeftClose, X } from 'lucide-react';
import type { Change, ChangePage, ChangeType } from '../../api/changes/client';
import type { Facet } from '../../api/types';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { formatLongDate, hhmm, timeAgo } from '../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { snapRange } from '../../lib/time';
import { token } from '../../theme/chartTheme';
import { CHANGE_TYPE_LABELS, CHANGE_TYPE_TOKENS, CHANGE_TYPES, changeFacetLabel, changeLink, sourceLabel } from './changeModel';
import { useChanges } from './queries';
import './infrastructure.css';

const PAGE_SIZE = 50;
const DETAIL_LABELS: Record<string, string> = {
  revision: 'Revision',
  replica_set: 'Replica set',
  change_cause: 'Change cause',
  generation: 'Generation',
  reason: 'Reason',
  version: 'Version',
  repository: 'Repository',
  pipeline: 'Pipeline',
  job: 'Job',
  status: 'Status',
  event_source: 'Event source',
  text: 'Text',
  action: 'Action',
  target: 'Target',
};

function TypeBadge({ type }: { type: ChangeType }) {
  return (
    <span className="ct-type">
      <span className="ct-dot" style={{ background: `var(${CHANGE_TYPE_TOKENS[type]})` }} />
      {CHANGE_TYPE_LABELS[type] ?? type}
    </span>
  );
}

function Histogram({ page }: { page: ChangePage | undefined }) {
  const option = useMemo<EChartsOption>(() => {
    const buckets = page?.histogram ?? [];
    const day = buckets.length > 1 && buckets[1].t - buckets[0].t >= 3 * 3600e3;
    return {
      grid: { left: 36, right: 12, top: 8, bottom: 20 },
      tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
      xAxis: {
        type: 'category',
        data: buckets.map((b) => String(b.t)),
        axisLabel: {
          formatter: (v: string) => (day ? `${new Date(Number(v)).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${hhmm(Number(v))}` : hhmm(Number(v))),
          interval: Math.max(0, Math.floor(buckets.length / 10) - 1),
        },
      },
      yAxis: { type: 'value', minInterval: 1, splitNumber: 2 },
      series: CHANGE_TYPES.map((t) => ({
        name: CHANGE_TYPE_LABELS[t],
        type: 'bar',
        stack: 'type',
        barCategoryGap: '25%',
        barMaxWidth: 14,
        color: token(CHANGE_TYPE_TOKENS[t]),
        data: buckets.map((b) => b.counts[t] ?? 0),
      })),
    };
  }, [page]);
  return <Chart option={option} height={110} />;
}

function toFacets(page: ChangePage | undefined): Facet[] {
  return (page?.facets ?? []).map((f) => ({ key: f.key, label: changeFacetLabel(f.key), group: ['change_type', 'source', 'resource_type'].includes(f.key) ? 'CHANGE' : 'TAGS', values: f.values }));
}

function ChangePanel({ change: c, onClose, onFilter }: { change: Change; onClose: () => void; onFilter: (k: string, v: string) => void }) {
  const navigate = useNavigate();
  const link = changeLink(c);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const field = (label: string, key: string, value: string | null | undefined) =>
    value ? (
      <>
        <dt>{label}</dt>
        <dd>
          <button onClick={() => onFilter(key, value)}>{value}</button>
        </dd>
      </>
    ) : null;
  return (
    <aside className="rc-panel" role="dialog" aria-label={`Change to ${c.resource}`}>
      <header>
        <TypeBadge type={c.changeType} />
        <span className="muted-text">via {sourceLabel(c.source)}</span>
        <button className="rc-panel-close" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <h2>{c.summary}</h2>
      <div className="rc-panel-actions">
        <span className="muted-text">{formatLongDate(c.timestamp)}</span>
        {link && (
          <button className="btn" onClick={() => navigate(link)}>
            <ExternalLink size={14} /> Open {c.resource}
          </button>
        )}
      </div>
      {(c.previous || c.current) && (
        <>
          <h3>What changed</h3>
          <div className="ct-diff">
            <div className="ct-diff-old">
              <span>−</span>
              <code>{c.previous ?? '—'}</code>
            </div>
            <div className="ct-diff-new">
              <span>+</span>
              <code>{c.current ?? '—'}</code>
            </div>
          </div>
        </>
      )}
      <h3>Resource</h3>
      <dl>
        {field('Resource', 'resource', c.resource)}
        {field('Resource type', 'resource_type', c.resourceType)}
        {field('Service', 'service', c.service)}
        {field('Environment', 'env', c.env)}
        {field('Cluster', 'cluster', c.cluster)}
        {field('Namespace', 'namespace', c.namespace)}
        {field('Author', 'author', c.author)}
      </dl>
      {Object.keys(c.details).length > 0 && (
        <>
          <h3>Details</h3>
          <dl>
            {Object.entries(c.details).map(([k, v]) => (
              <Fragment key={k}>
                <dt>{DETAIL_LABELS[k] ?? k}</dt>
                <dd className="rc-wrap">{v}</dd>
              </Fragment>
            ))}
          </dl>
        </>
      )}
    </aside>
  );
}

/** Infrastructure → Resources → Changes: rollouts, scaling, config edits, deployments and crashes over time. */
export function ChangesPage() {
  return (
    <TimeProvider initial="1w">
      <ChangesBody />
    </TimeProvider>
  );
}

function ChangesBody() {
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const offset = Number(params.get('offset') ?? 0);
  const [showFacets, setShowFacets] = useState(true);
  const [selected, setSelected] = useState<Change | null>(null);

  const setParam = useCallback(
    (updates: Record<string, string | null>) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(updates)) {
            if (v === null || v === '') next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );
  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((t: Term[]) => setParam({ q: formatQuery(t), offset: null }), [setParam]);
  const addFilter = useCallback((k: string, v: string) => setParam({ q: addTerm(query, k, v), offset: null }), [setParam, query]);

  const q = useChanges({ range, query, offset, limit: PAGE_SIZE });
  const page = q.data;
  const total = page?.total ?? 0;
  const facets = useMemo(() => toFacets(page), [page]);

  return (
    <div className="rc-page">
      <div className="section-header et-section-header">
        <div className="section-header-left">
          <div className="page-tab active">Change Tracking</div>
        </div>
        <TimePicker compact />
      </div>

      <div className="hosts-scroll">
        <QueryBar value={query} onChange={(v) => setParam({ q: v, offset: null })} placeholder="Filter changes (e.g. change_type:rollout service:order-service env:prod)" />

        <div className="ct-histogram">
          <Histogram page={page} />
        </div>

        <div className="hosts-body">
          {showFacets && page && (
            <FacetPanel
              facets={facets}
              terms={terms}
              onChange={setTerms}
              mode="include"
              valueSearch
              defaultOpen={['change_type', 'service', 'env', 'cluster']}
              noun="changes"
              renderValueIcon={(key, value) =>
                key === 'change_type' && value in CHANGE_TYPE_TOKENS ? <span className="ct-dot" style={{ background: `var(${CHANGE_TYPE_TOKENS[value as ChangeType]})` }} /> : null
              }
            />
          )}

          <div className="hosts-main">
            <div className="hosts-toolbar">
              <button className="link-btn" onClick={() => setShowFacets((s) => !s)}>
                {showFacets ? <PanelLeftClose size={15} /> : <PanelLeft size={15} />}
                {showFacets ? 'Hide Facets' : 'Show Facets'}
              </button>
              <span className="hosts-showing">
                <strong>{total.toLocaleString()}</strong> Changes
              </span>
            </div>

            {q.isPending ? (
              <Loading label="Loading changes…" />
            ) : q.isError ? (
              <ErrorState error={q.error} onRetry={() => q.refetch()} />
            ) : !page || page.items.length === 0 ? (
              <Empty>
                No changes in this time range. Changes come from Kubernetes (deployment rollouts, scaling and spec edits, pod crashes) sent by the cluster collector or the Datadog Cluster Agent, new
                service versions seen in APM traces, CI deployment jobs, deploy, config and feature flag events sent to the events API, and configuration edits in ObserveCraft (organization admins).
              </Empty>
            ) : (
              <div className={q.isPlaceholderData ? 'is-stale' : undefined}>
                <table className="host-table rc-table ct-table">
                  <thead>
                    <tr>
                      <th>TIME</th>
                      <th>CHANGE TYPE</th>
                      <th>RESOURCE</th>
                      <th>SERVICE</th>
                      <th>ENVIRONMENT</th>
                      <th>SUMMARY</th>
                      <th>AUTHOR</th>
                    </tr>
                  </thead>
                  <tbody>
                    {page.items.map((c) => (
                      <tr key={c.id} className={selected?.id === c.id ? 'selected' : undefined} onClick={() => setSelected(c)}>
                        <td title={formatLongDate(c.timestamp)}>{timeAgo(c.timestamp)}</td>
                        <td>
                          <TypeBadge type={c.changeType} />
                        </td>
                        <td className="rc-resource">
                          <span>
                            <span className="rc-name" title={c.resource}>
                              {c.resource}
                            </span>
                            <span className="rc-subtype">
                              {c.resourceType} · {sourceLabel(c.source)}
                            </span>
                          </span>
                        </td>
                        <td>{c.service ? <span className="tag-chip">{c.service}</span> : <span className="muted-text">—</span>}</td>
                        <td>{c.env ? <span className="tag-chip">{c.env}</span> : <span className="muted-text">—</span>}</td>
                        <td className="ct-summary" title={c.summary}>
                          {c.summary}
                        </td>
                        <td>{c.author ?? <span className="muted-text">—</span>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                <Pager offset={offset} limit={PAGE_SIZE} total={total} onChange={(o) => setParam({ offset: o ? String(o) : null })} />
              </div>
            )}
          </div>
        </div>
      </div>

      {selected && <ChangePanel change={selected} onClose={() => setSelected(null)} onFilter={addFilter} />}
    </div>
  );
}

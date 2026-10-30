import { Fragment, useCallback, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { ChevronDown, ChevronRight, Copy, ExternalLink, LayoutList, Search, X } from 'lucide-react';
import type { CatalogType } from '../../api/catalog/client';
import type { JsonValue } from '../../api/types';
import { searchResourceChanges, type ResourceChange } from '../../api/resourceChanges/client';
import { Chart } from '../../components/charts/Chart';
import { JsonTree } from '../../components/json/JsonTree';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { timeAgo } from '../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { snapRange } from '../../lib/time';
import { palette } from '../../theme/chartTheme';
import { TypeIcon, valueLabel } from './CatalogPanel';
import { FilterDropdown } from './ResourceCatalogPage';
import './infrastructure.css';

const PAGE_SIZE = 50;
const NOISE = '@is_noise:false';
const INTRO_KEY = 'oc.resource-changes.intro-dismissed';
/** Header filters and their labels, in display order. */
const FILTERS: [string, string][] = [
  ['team', 'Team'],
  ['risk', 'Risk Level'],
  ['cloud', 'Cloud'],
  ['region', 'Region'],
  ['env', 'Environment'],
  ['account', 'Account'],
  ['service', 'Service'],
  ['change_type', 'Change Types'],
];
const GROUP_BY = ['resource_type', 'risk', 'change_type', 'team', 'service', 'env', 'region'];

const readIntro = () => {
  try {
    return localStorage.getItem(INTRO_KEY) === '1';
  } catch {
    return false;
  }
};

function RiskBadge({ risk, small }: { risk: string; small?: boolean }) {
  return <span className={`rch-risk rch-risk-${risk}${small ? ' rch-risk-sm' : ''}`}>{risk.toUpperCase()}</span>;
}

/** The first few values as chips, then "+N". */
function Chips({ values, max, mono }: { values: string[]; max: number; mono?: boolean }) {
  return (
    <span className="rch-chips">
      {values.slice(0, max).map((v) => (
        <span key={v} className={`rch-chip${mono ? ' rch-chip-field' : ''}`} title={v}>
          {v}
        </span>
      ))}
      {values.length > max && (
        <span className="rc-more" title={values.slice(max).join(', ')}>
          +{values.length - max}
        </span>
      )}
    </span>
  );
}

/** A path's value in a flattened configuration (`attributes.images`, `…containers[app].image`). */
function flatten(prefix: string, v: unknown, out: Map<string, string>) {
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v)) flatten(prefix ? `${prefix}.${k}` : k, x, out);
  } else if (Array.isArray(v) && v.length && v.every((i) => i && typeof i === 'object' && typeof (i as { name?: unknown }).name === 'string')) {
    for (const i of v as { name: string }[]) flatten(`${prefix}[${i.name}]`, i, out);
  } else if (v !== null && v !== undefined) {
    out.set(prefix, typeof v === 'string' ? v : JSON.stringify(v));
  }
}

function ChangeDetail({ change: c, onResource }: { change: ResourceChange; onResource: () => void }) {
  const [filter, setFilter] = useState('');
  const shown = (c.newFields ?? c.oldFields) as Record<string, JsonValue> | null | undefined;
  const changed = c.changeTypes.includes('creation') || c.changeTypes.includes('deletion') ? [] : c.changedFields;
  const before = new Map<string, string>();
  const after = new Map<string, string>();
  flatten('', c.oldFields, before);
  flatten('', c.newFields, after);
  return (
    <div className="rch-detail">
      <div className="rch-detail-top">
        {c.resourceChanges > 1 ? (
          <button className="link-btn" onClick={onResource}>
            View {c.resourceChanges} changes to this resource in the selected timeframe →
          </button>
        ) : (
          <span />
        )}
      </div>
      <h4>
        <RiskBadge risk={c.risk} small /> {c.title}
      </h4>
      <p>{c.description}</p>
      {changed.length > 0 && (
        <table className="rch-diff">
          <thead>
            <tr>
              <th>FIELD</th>
              <th>BEFORE</th>
              <th>AFTER</th>
            </tr>
          </thead>
          <tbody>
            {changed.map((f) => (
              <tr key={f}>
                <td className="mono">{f}</td>
                <td className="mono rch-old">{before.get(f) ?? '—'}</td>
                <td className="mono rch-new">{after.get(f) ?? '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="muted-text">
        {c.newFields ? (
          <>
            Showing <code>@new_fields</code> configuration data.
          </>
        ) : (
          <>
            No <code>@new_fields</code> detected. Showing <code>@old_fields</code> configuration data.
          </>
        )}
      </p>
      <div className="rch-json-bar">
        <label className="tv-filter">
          <Search size={14} />
          <input placeholder="Filter JSON" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
        <button className="icon-plain" title="Clear" onClick={() => setFilter('')}>
          <X size={16} />
        </button>
        <button className="icon-plain" title="Copy JSON" onClick={() => navigator.clipboard?.writeText(JSON.stringify(shown ?? {}, null, 2))}>
          <Copy size={16} />
        </button>
      </div>
      <div className="rch-json">{shown ? <JsonTree data={shown} filter={filter} expandDepth={2} /> : <span className="muted-text">No configuration recorded.</span>}</div>
    </div>
  );
}

function Body() {
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? NOISE;
  const groupBy = params.get('group') ?? 'resource_type';
  const offset = Number(params.get('offset') ?? 0);
  const [intro, setIntro] = useState(!readIntro());
  const [open, setOpen] = useState<string | null>(null);
  const setParam = useCallback(
    (u: Record<string, string | null>) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(u)) {
            if (v === null) next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );
  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = (t: Term[]) => setParam({ q: formatQuery(t), offset: null });
  const noise = /(^|\s)@is_noise:false(\s|$)/.test(query);
  const q = useQuery({
    queryKey: ['infrastructure', 'resource-changes', range, query, groupBy, offset],
    queryFn: ({ signal }) => searchResourceChanges({ range, query, groupBy, offset, limit: PAGE_SIZE }, signal),
    placeholderData: keepPreviousData,
  });
  const page = q.data;
  const facet = (k: string) => page?.facets.find((f) => f.key === k);
  const groups = useMemo(() => {
    const totals = new Map<string, number>();
    for (const b of page?.histogram ?? []) for (const [k, n] of Object.entries(b.counts)) totals.set(k, (totals.get(k) ?? 0) + n);
    return [...totals].sort((a, b) => b[1] - a[1]).map(([k]) => k);
  }, [page]);
  const colors = palette();
  const option: EChartsOption = {
    grid: { left: 8, right: 12, top: 12, bottom: 50, containLabel: true },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
    legend: { bottom: 0, type: 'scroll', formatter: (n: string) => valueLabel(groupBy, n) },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', minInterval: 1 },
    series: groups.map((g, i) => ({
      name: g,
      type: 'bar',
      stack: 'changes',
      barMaxWidth: 60,
      color: colors[i % colors.length],
      data: (page?.histogram ?? []).map((b) => [b.t + (page?.bucketMs ?? 0) / 2, b.counts[g] ?? 0]),
    })),
  };
  const toggleNoise = () => setParam({ q: noise ? query.replace(/(^|\s)@is_noise:false(?=\s|$)/, '').trim() : `${NOISE} ${query}`.trim(), offset: null });

  return (
    <div className="rch-page">
      {intro && (
        <section className="rch-intro">
          <div>
            <div className="rch-intro-title">
              <LayoutList size={18} /> Resource Changes <span className="rc-preview">PREVIEW</span>
            </div>
            <h2>Stay on top of your infrastructure changes</h2>
            <p>Track and review what changed in the configuration of your hosts, Kubernetes objects and ECS resources.</p>
            <Link className="btn" to="/infrastructure/catalog">
              View the Resource Catalog <ExternalLink size={14} />
            </Link>
          </div>
          <div className="rch-intro-cards">
            <div>
              <b>30 DAYS OF HISTORY</b>
              <p>Every change of the past 30 days, with the configuration before and after it.</p>
            </div>
            <div>
              <b>AUTOMATIC SNAPSHOTS</b>
              <p>Resources are compared with their last snapshot every 5 minutes — no setup required.</p>
            </div>
            <div>
              <b>SUPPORTED RESOURCES</b>
              <p>Hosts, every Kubernetes object of the Resource Catalog, and ECS clusters, services and tasks.</p>
            </div>
          </div>
          <button
            className="icon-plain rch-intro-close"
            title="Dismiss"
            onClick={() => {
              setIntro(false);
              try {
                localStorage.setItem(INTRO_KEY, '1');
              } catch {
                /* storage unavailable */
              }
            }}
          >
            <X size={18} />
          </button>
        </section>
      )}
      <div className="rch-filters">
        {FILTERS.map(([k, label]) => (
          <FilterDropdown key={k} facetKey={k} label={label} facet={facet(k)} terms={terms} onChange={setTerms} />
        ))}
        <span className="rch-push">
          <TimePicker />
        </span>
      </div>
      <div className="rch-search">
        <FilterDropdown facetKey="resource_type" label="Resources" facet={facet('resource_type')} terms={terms} onChange={setTerms} />
        <div className="rch-query">
          <QueryBar value={query} onChange={(v) => setParam({ q: v, offset: null })} placeholder="Search changes (e.g. risk:high change_type:deletion)" />
        </div>
        <label className="rch-noise">
          <button role="switch" aria-checked={noise} className={`switch${noise ? ' on' : ''}`} onClick={toggleNoise} />
          Noise Reduction
        </label>
      </div>
      <section className="rch-chart">
        <div className="rch-chart-head">
          Changes by
          <Select value={groupBy} options={GROUP_BY.map((g) => ({ value: g, label: g }))} onChange={(g) => setParam({ group: g })} />
        </div>
        <div className="rch-chart-body">{page && <Chart option={option} height="100%" />}</div>
      </section>
      {q.isPending ? (
        <Loading label="Loading changes…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !page?.total ? (
        <Empty>No resource changed in this time frame, or none matches the filters. Changes are detected every 5 minutes.</Empty>
      ) : (
        <>
          <div className="rch-count">
            <b>{page.total.toLocaleString()}</b> Resource Change Events matched your query
          </div>
          <div className="rch-table-wrap">
            <table className="rch-table">
              <thead>
                <tr>
                  <th />
                  <th>RISK LEVEL</th>
                  <th>SUMMARY</th>
                  <th>@CHANGE_TYPES</th>
                  <th>@CHANGED_FIELDS</th>
                  <th>RESOURCE DETAILS</th>
                  <th />
                  <th>DETECTED</th>
                </tr>
              </thead>
              <tbody>
                {page.items.map((c) => {
                  const key = `${c.detected}:${c.resourceId}`;
                  const expanded = open === key;
                  return (
                    <Fragment key={key}>
                      <tr className="rch-row" onClick={() => setOpen(expanded ? null : key)}>
                        <td className="rch-toggle">{expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</td>
                        <td>
                          <RiskBadge risk={c.risk} />
                        </td>
                        <td className="rch-summary">
                          <span className="rch-resource">
                            <TypeIcon type={c.resourceType as CatalogType} size={14} />
                            <b>{valueLabel('resource_type', c.resourceType)}</b>
                            <span className="muted-text" title={c.resourceName}>
                              {c.resourceName}
                            </span>
                          </span>
                          <span>{c.title}</span>
                        </td>
                        <td>
                          <Chips values={c.changeTypes} max={3} />
                        </td>
                        <td className="rch-fields">
                          <Chips values={c.changedFields} max={4} mono />
                        </td>
                        <td className="rch-details">
                          {c.team ? <span className="rch-pill">{c.team}</span> : <span className="muted-text">No Team</span>}
                          <span className="muted-text">{c.account || 'No account'}</span>
                        </td>
                        <td className="rch-details">
                          <span className="muted-text">{c.service || 'No Service'}</span>
                          {c.region ? <span className="rch-chip">region:{c.region}</span> : c.cluster ? <span className="rch-chip">cluster:{c.cluster}</span> : null}
                        </td>
                        <td className="muted-text" title={new Date(c.detected).toLocaleString()}>
                          ~{timeAgo(c.detected)}
                        </td>
                      </tr>
                      {expanded && (
                        <tr className="rch-expanded">
                          <td colSpan={8}>
                            <ChangeDetail change={c} onResource={() => setParam({ q: `${query} resource_id:${c.resourceId}`.trim(), offset: null })} />
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
          <Pager offset={offset} limit={PAGE_SIZE} total={page.total} onChange={(o) => setParam({ offset: String(o) })} />
        </>
      )}
    </div>
  );
}

/** Infrastructure → Resources → Changes: what changed in the configuration of the catalog's resources. */
export function ResourceChangesPage() {
  return (
    <TimeProvider initial="1d">
      <Body />
    </TimeProvider>
  );
}

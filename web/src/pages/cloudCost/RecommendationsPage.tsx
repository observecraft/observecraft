import { useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Link, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowDown, ArrowRight, ArrowUp, Cloud, Lightbulb, Search, X } from 'lucide-react';
import { costRecommendations, type CostRecommendation, type RecommendationGroup, type RecommendationsResult } from '../../api/cloudCost/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { palette, token } from '../../theme/chartTheme';
import { money, providerLabel } from './costModel';
import { ProviderIcon } from './widgets';

type View = 'overview' | 'list';
type SortKey = 'resource' | 'type' | 'recommendation' | 'account' | 'region' | 'cost' | 'savings';

/** What the list and the donut can be grouped by; `tag:<key>` groups by a tag's value. */
const GROUP_BYS: { value: string; label: string }[] = [
  { value: '', label: 'None' },
  { value: 'recommendationType', label: 'Recommendation Type' },
  { value: 'resourceType', label: 'Resource Type' },
  { value: 'provider', label: 'Provider' },
  { value: 'account', label: 'Account' },
  { value: 'region', label: 'Region' },
];

function groupOf(r: CostRecommendation, by: string): string {
  if (by.startsWith('tag:')) {
    const key = by.slice(4);
    return r.tags.find((t) => t.startsWith(`${key}:`))?.slice(key.length + 1) ?? `No ${key}`;
  }
  switch (by) {
    case 'resourceType':
      return r.resourceTypeLabel;
    case 'provider':
      return providerLabel(r.provider);
    case 'account':
      return r.account || 'Unknown account';
    case 'region':
      return r.region || 'Unknown region';
    default:
      return r.recommendationLabel;
  }
}

/** Recommendations grouped by `by`, largest savings first. */
function grouped(recs: CostRecommendation[], by: string): { name: string; recs: CostRecommendation[]; cost: number; savings: number }[] {
  const map = new Map<string, CostRecommendation[]>();
  for (const r of recs) map.set(groupOf(r, by), [...(map.get(groupOf(r, by)) ?? []), r]);
  return [...map.entries()]
    .map(([name, rs]) => ({ name, recs: rs, cost: rs.reduce((s, r) => s + r.monthlyCost, 0), savings: rs.reduce((s, r) => s + r.monthlySavings, 0) }))
    .sort((a, b) => b.savings - a.savings || a.name.localeCompare(b.name));
}

function donut(groups: { name: string; savings: number }[], total: number): EChartsOption {
  const colors = palette();
  return {
    tooltip: { trigger: 'item', confine: true, valueFormatter: (v) => money(Number(v), { precise: true }) },
    title: { text: money(total), subtext: 'a month', left: 'center', top: '43%', textStyle: { fontSize: 20, fontWeight: 600, color: token('--text') }, subtextStyle: { color: token('--text-muted') } },
    series: [
      {
        type: 'pie',
        radius: ['42%', '64%'],
        avoidLabelOverlap: true,
        label: { formatter: (p) => `${p.name}\n${money(Number(p.value))}`, color: token('--text'), fontSize: 11, width: 130, overflow: 'break' },
        labelLine: { length: 10, length2: 8 },
        data: groups.map((g, i) => ({ name: g.name, value: Math.round(g.savings * 100) / 100, itemStyle: { color: colors[i % colors.length] } })),
      },
    ],
  };
}

function NoAccounts() {
  return (
    <div className="ccm-rec-empty">
      <Cloud size={30} />
      <h2>Connect an AWS or Azure account</h2>
      <p>
        Recommendations come from the CloudWatch and Azure Monitor metrics ObserveCraft collects for your accounts: idle instances and VMs, unused databases, idle volumes and disks, unused NAT
        gateways, load balancers and public IP addresses.
      </p>
      <div className="ccm-rec-empty-actions">
        <Link className="btn btn-primary" to="/integrations/aws">
          <ProviderIcon provider="aws" size={16} /> Connect AWS
        </Link>
        <Link className="btn" to="/integrations/azure">
          <ProviderIcon provider="azure" size={16} /> Connect Azure
        </Link>
      </div>
    </div>
  );
}

function NothingFound({ days, filtered, onClear }: { days: number; filtered: boolean; onClear: () => void }) {
  if (filtered)
    return (
      <Empty>
        No recommendations match these filters.{' '}
        <button className="link-btn" onClick={onClear}>
          Clear filters
        </button>
      </Empty>
    );
  return (
    <div className="ccm-rec-empty">
      <Lightbulb size={30} />
      <h2>No waste found</h2>
      <p>
        None of the resources in your connected accounts looked idle or over-provisioned over the last {days} days. Resources need a few days of metrics before they are judged; new accounts fill in as
        the crawlers collect.
      </p>
    </div>
  );
}

function Overview({ data, groupBy, onPickType }: { data: RecommendationsResult; groupBy: string; onPickType: (id: string) => void }) {
  const slices = grouped(data.recommendations, groupBy || 'recommendationType');
  const top = slices.slice(0, 7);
  const rest = slices.slice(7);
  const donutGroups = rest.length ? [...top, { name: 'Other', savings: rest.reduce((s, g) => s + g.savings, 0) }] : top;
  const groupLabel = GROUP_BYS.find((g) => g.value === (groupBy || 'recommendationType'))?.label ?? groupBy.replace(/^tag:/, '');
  return (
    <div className="ccm-rec-overview">
      <div className="ccm-cm-kpis">
        <div className="ccm-cm-kpi active">
          <span className="ccm-dotted" title="What acting on every recommendation would save each month">
            Potential Monthly Savings
          </span>
          <b className="ccm-down">{money(data.monthlySavings)}</b>
        </div>
        <div className="ccm-cm-kpi">
          <span>Potential Annual Savings</span>
          <b>{money(data.annualSavings)}</b>
        </div>
        <div className="ccm-cm-kpi">
          <span>Recommendations</span>
          <b>{data.count.toLocaleString('en-US')}</b>
          <small className="muted-text">on resources costing {money(data.monthlyCost)} a month</small>
        </div>
      </div>
      <div className="ccm-widget">
        <div className="ccm-widget-head">
          <span>Top recommendations by {groupLabel.toLowerCase()}</span>
          <span className="ccm-badge">{data.lookbackDays}d</span>
        </div>
        <div className="ccm-rec-donut">
          <Chart height="100%" option={donut(donutGroups, data.monthlySavings)} />
        </div>
      </div>
      <div className="ccm-rec-cards">
        {data.byType.slice(0, 5).map((g: RecommendationGroup) => (
          <button key={g.id} className="ccm-rec-card" onClick={() => onPickType(g.id)}>
            <span className="ccm-rec-card-title">
              {g.label} <span className="ccm-badge">{g.count}</span>
            </span>
            <span className="ccm-rec-card-figures">
              <span>
                <small className="muted-text">Est. monthly cost</small>
                <b>{money(g.monthlyCost)}</b>
              </span>
              <span>
                <small className="muted-text">Potential monthly savings</small>
                <b className="ccm-down">{money(g.monthlySavings)}</b>
              </span>
            </span>
            <span className="ccm-rec-card-link">
              View all <ArrowRight size={13} />
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

const COLUMNS: { key: SortKey; label: string; num?: boolean }[] = [
  { key: 'resource', label: 'RESOURCE' },
  { key: 'type', label: 'RESOURCE TYPE' },
  { key: 'recommendation', label: 'RECOMMENDATION' },
  { key: 'account', label: 'ACCOUNT' },
  { key: 'region', label: 'REGION' },
  { key: 'cost', label: 'EST. MONTHLY COST', num: true },
  { key: 'savings', label: 'POTENTIAL SAVINGS', num: true },
];

function sortValue(r: CostRecommendation, key: SortKey): string | number {
  switch (key) {
    case 'resource':
      return r.resourceName.toLowerCase();
    case 'type':
      return r.resourceTypeLabel;
    case 'recommendation':
      return r.recommendationLabel;
    case 'account':
      return r.account;
    case 'region':
      return r.region;
    case 'cost':
      return r.monthlyCost;
    default:
      return r.monthlySavings;
  }
}

function Row({ r, selected, onOpen }: { r: CostRecommendation; selected: boolean; onOpen: () => void }) {
  return (
    <tr className={`ccm-clickable${selected ? ' ccm-rec-selected' : ''}`} onClick={onOpen}>
      <td>
        <div className="ccm-rec-resource">
          <b>{r.resourceName}</b>
          {r.resourceName !== r.resourceId.split('/').pop() && (
            <span className="ccm-mono muted-text" title={r.resourceId}>
              {r.resourceId.split('/').pop()}
            </span>
          )}
        </div>
      </td>
      <td>
        <span className="ccm-rec-type">
          <ProviderIcon provider={r.provider} size={16} /> {r.resourceTypeLabel}
        </span>
      </td>
      <td>{r.recommendationLabel}</td>
      <td className="ccm-mono ccm-rec-account" title={r.account}>
        {r.account}
      </td>
      <td>{r.region}</td>
      <td className="num" title={r.costSource === 'list_price' ? `Estimate: ${r.costBasis}` : 'From your bills, last 30 days'}>
        {money(r.monthlyCost)}
        {r.costSource === 'list_price' && <span className="muted-text"> est.</span>}
      </td>
      <td className="num ccm-down">{money(r.monthlySavings)}</td>
    </tr>
  );
}

function List({ data, groupBy, selected, onOpen }: { data: RecommendationsResult; groupBy: string; selected: string; onOpen: (id: string) => void }) {
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'savings', desc: true });
  const sorted = useMemo(() => {
    const dir = sort.desc ? -1 : 1;
    return [...data.recommendations].sort((a, b) => {
      const x = sortValue(a, sort.key);
      const y = sortValue(b, sort.key);
      return (typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y))) * dir || a.id.localeCompare(b.id);
    });
  }, [data.recommendations, sort]);
  const toggle = (key: SortKey) => setSort((s) => (s.key === key ? { key, desc: !s.desc } : { key, desc: key === 'cost' || key === 'savings' }));
  return (
    <div className="ccm-rec-list">
      <table className="ccm-table">
        <thead>
          <tr>
            {COLUMNS.map((c) => (
              <th key={c.key} className={c.num ? 'num' : ''}>
                <button className="ccm-rec-sort" onClick={() => toggle(c.key)}>
                  {c.label} {sort.key === c.key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
                </button>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {groupBy
            ? grouped(sorted, groupBy).map((g) => [
                <tr key={`group:${g.name}`} className="ccm-rec-group">
                  <td colSpan={5}>
                    {g.name} <span className="ccm-badge">{g.recs.length}</span>
                  </td>
                  <td className="num">{money(g.cost)}</td>
                  <td className="num ccm-down">{money(g.savings)}</td>
                </tr>,
                ...g.recs.map((r) => <Row key={r.id} r={r} selected={r.id === selected} onOpen={() => onOpen(r.id)} />),
              ])
            : sorted.map((r) => <Row key={r.id} r={r} selected={r.id === selected} onOpen={() => onOpen(r.id)} />)}
        </tbody>
      </table>
    </div>
  );
}

function Drawer({ r, onClose }: { r: CostRecommendation; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <>
      <div className="ccm-rec-backdrop" onClick={onClose} />
      <aside className="ccm-rec-drawer" aria-label={`Recommendation for ${r.resourceName}`}>
        <header className="ccm-rec-drawer-head">
          <ProviderIcon provider={r.provider} size={22} />
          <div>
            <h2>{r.resourceName}</h2>
            <span className="muted-text">{r.resourceTypeLabel}</span>
          </div>
          <button className="icon-btn" title="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="ccm-rec-drawer-body">
          <section>
            <h3>{r.recommendationLabel}</h3>
            <p>{r.reason}.</p>
          </section>
          <section className="ccm-rec-figures">
            <div>
              <small className="muted-text">Est. monthly cost</small>
              <b>{money(r.monthlyCost)}</b>
            </div>
            <div>
              <small className="muted-text">Potential monthly savings</small>
              <b className="ccm-down">{money(r.monthlySavings)}</b>
            </div>
            <div>
              <small className="muted-text">Potential annual savings</small>
              <b className="ccm-down">{money(r.monthlySavings * 12)}</b>
            </div>
          </section>
          <p className="ccm-rec-note muted-text">
            {r.costSource === 'bill'
              ? 'Cost from your imported bills over the last 30 days.'
              : `Estimated from list prices: ${r.costBasis}. Import your bills in Cloud Cost → Settings for exact costs.`}
          </p>
          <section>
            <h4>Evidence</h4>
            <dl className="ccm-rec-evidence">
              {r.evidence.map((e) => (
                <div key={e.label}>
                  <dt>{e.label}</dt>
                  <dd>{e.value}</dd>
                </div>
              ))}
              <div>
                <dt>Days with data</dt>
                <dd>{Math.round(r.observedDays)}</dd>
              </div>
            </dl>
          </section>
          <section>
            <h4>How to act</h4>
            <p>{r.action}</p>
          </section>
          <section>
            <h4>Resource</h4>
            <dl className="ccm-rec-evidence">
              <div>
                <dt>ID</dt>
                <dd className="ccm-mono ccm-rec-id">{r.resourceId}</dd>
              </div>
              <div>
                <dt>{r.provider === 'aws' ? 'Account' : 'Subscription'}</dt>
                <dd className="ccm-mono">{r.account}</dd>
              </div>
              <div>
                <dt>Region</dt>
                <dd>{r.region}</dd>
              </div>
            </dl>
            {r.tags.length > 0 && (
              <div className="ccm-chips">
                {r.tags.map((t) => (
                  <span key={t} className="ccm-chip">
                    {t}
                  </span>
                ))}
              </div>
            )}
          </section>
        </div>
      </aside>
    </>
  );
}

const parseTags = (text: string) =>
  text
    .split(/[\s,]+/)
    .map((t) => t.trim())
    .filter(Boolean);

/** Optimize → Recommendations: idle and over-provisioned AWS and Azure resources, with what acting on them saves. */
export function RecommendationsPage() {
  const [params, setParams] = useSearchParams();
  const view: View = params.get('view') === 'list' ? 'list' : 'overview';
  const resourceType = params.get('resourceType') ?? '';
  const recommendationType = params.get('recommendationType') ?? '';
  const tagsText = params.get('tags') ?? '';
  const groupBy = params.get('groupBy') ?? '';
  const selected = params.get('rec') ?? '';
  const [tagInput, setTagInput] = useState(tagsText);
  useEffect(() => setTagInput(tagsText), [tagsText]);

  const set = (changes: Record<string, string>) =>
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        for (const [k, v] of Object.entries(changes)) v ? next.set(k, v) : next.delete(k);
        return next;
      },
      { replace: true },
    );
  const tags = parseTags(tagsText);
  const q = useQuery({
    queryKey: ['cloud-cost', 'recommendations', resourceType, recommendationType, tags.join(',')],
    queryFn: ({ signal }) => costRecommendations({ resourceType, recommendationType, tags }, signal),
    placeholderData: (prev) => prev,
  });
  const data = q.data;
  const filtered = Boolean(resourceType || recommendationType || tags.length);
  const tagKeys = useMemo(() => [...new Set((data?.recommendations ?? []).flatMap((r) => r.tags.map((t) => t.split(':')[0])))].sort(), [data]);
  const open = data?.recommendations.find((r) => r.id === selected);

  return (
    <div className="ccm-page">
      <div className="ccm-filterbar">
        <Select
          label="Resource Type"
          value={resourceType}
          options={[{ value: '', label: 'All' }, ...(data?.resourceTypes ?? []).map((g) => ({ value: g.id, label: g.label }))]}
          onChange={(v) => set({ resourceType: v })}
          triggerClassName="dd-trigger ccm-filter"
        />
        <Select
          label="Recommendation Type"
          value={recommendationType}
          options={[{ value: '', label: 'All' }, ...(data?.recommendationTypes ?? []).map((g) => ({ value: g.id, label: g.label }))]}
          onChange={(v) => set({ recommendationType: v })}
          triggerClassName="dd-trigger ccm-filter ccm-rec-filter-wide"
        />
        <form
          className="ccm-search ccm-rec-tags"
          onSubmit={(e) => {
            e.preventDefault();
            set({ tags: parseTags(tagInput).join(' ') });
          }}
        >
          <Search size={14} />
          <input
            placeholder="Filter by tags: env:prod team:storefront region:us-east-1"
            value={tagInput}
            onChange={(e) => setTagInput(e.target.value)}
            onBlur={() => set({ tags: parseTags(tagInput).join(' ') })}
            aria-label="Filter by tags"
          />
        </form>
        <Select
          label="Group By"
          value={groupBy}
          options={[...GROUP_BYS, ...tagKeys.map((k) => ({ value: `tag:${k}`, label: k }))]}
          onChange={(v) => set({ groupBy: v })}
          triggerClassName="dd-trigger ccm-filter"
        />
        <span className="ccm-filterbar-right">
          <span className="btn-group">
            <button className={`btn btn-sm${view === 'overview' ? ' btn-primary' : ''}`} onClick={() => set({ view: '' })}>
              Overview
            </button>
            <button className={`btn btn-sm${view === 'list' ? ' btn-primary' : ''}`} onClick={() => set({ view: 'list' })}>
              All Recommendations{data ? ` (${data.count})` : ''}
            </button>
          </span>
        </span>
      </div>
      <div className="ccm-rec-body">
        {q.isPending ? (
          <Loading label="Looking for savings…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : data!.accounts.aws + data!.accounts.azure === 0 ? (
          <NoAccounts />
        ) : data!.count === 0 ? (
          <NothingFound days={data!.lookbackDays} filtered={filtered} onClear={() => set({ resourceType: '', recommendationType: '', tags: '' })} />
        ) : view === 'list' ? (
          <List data={data!} groupBy={groupBy} selected={selected} onOpen={(id) => set({ rec: id })} />
        ) : (
          <Overview data={data!} groupBy={groupBy} onPickType={(id) => set({ view: 'list', recommendationType: id })} />
        )}
      </div>
      {open && <Drawer r={open} onClose={() => set({ rec: '' })} />}
    </div>
  );
}

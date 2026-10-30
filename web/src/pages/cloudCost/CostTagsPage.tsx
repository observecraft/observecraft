import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ChevronDown, CircleDollarSign, Search } from 'lucide-react';
import type { CostType } from '../../api/cloudCost/client';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { Select } from '../../components/ui/Dropdown';
import { DAY, isCloud, monthName, monthStart, providerLabel, today, useCostTags } from './costModel';
import { ProviderIcon, ShareBar } from './widgets';

const SOURCES: { group: string; items: { id: string; label: string }[] }[] = [
  { group: 'Cloud Cost Management', items: [{ id: 'focus', label: 'FOCUS' }] },
  {
    group: 'Providers',
    items: [
      { id: 'aws', label: 'AWS' },
      { id: 'azure', label: 'Azure' },
      { id: 'gcp', label: 'Google Cloud' },
    ],
  },
  { group: 'Containers', items: [{ id: 'kubernetes', label: 'Kubernetes' }] },
  { group: 'Resources', items: [{ id: 'resource', label: 'Resource tags' }] },
];
const CHIPS = 6;

function SourceIcon({ source }: { source: string }) {
  if (isCloud(source)) return <ProviderIcon provider={source} />;
  if (source === 'kubernetes') return <BrandLogo name="kubernetes" size={18} title="Kubernetes" />;
  return <CircleDollarSign size={18} className="ccm-source-icon" />;
}

/** Analyze → Tags: the tags costs break down by, where they come from and how much cost they cover. */
export function CostTagsPage() {
  const now = today();
  const [provider, setProvider] = useState('');
  const [month, setMonth] = useState(String(monthStart(now)));
  const [costType, setCostType] = useState<CostType>('amortized');
  const [search, setSearch] = useState('');
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [sourcesOpen, setSourcesOpen] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);
  const months = useMemo(() => Array.from({ length: 13 }, (_, i) => monthStart(now, -i)), [now]);
  const from = Number(month);
  const range = useMemo(() => ({ from, to: Math.min(monthStart(from, 1), now + DAY) - 1 }), [from, now]);
  const providers = useCostTags(range);
  const tags = useCostTags(range, provider ? `providername:"${provider}"` : '', costType);
  const rows = (tags.data ?? []).filter((t) => !hidden.has(t.source) && (!search || t.key.toLowerCase().includes(search.toLowerCase())));
  return (
    <div className="ccm-page">
      <p className="ccm-intro">Every tag your costs can be broken down by: the bills' standard columns, the providers' own tags and the tags on the billed resources.</p>
      <div className="ccm-tags-filters">
        <Select
          label="Provider"
          value={provider}
          options={[
            { value: '', label: 'All' },
            ...(providers.data?.find((t) => t.key === 'providername')?.values ?? []).map((v) => ({
              value: v.value,
              label: providerLabel(v.value),
              icon: <ProviderIcon provider={v.value} size={16} />,
            })),
          ]}
          onChange={setProvider}
          triggerClassName="dd-trigger ccm-filter"
        />
        <Select
          label="Month"
          value={month}
          options={months.map((m) => ({ value: String(m), label: `${monthName(m)} ${new Date(m).getUTCFullYear()}` }))}
          onChange={setMonth}
          triggerClassName="dd-trigger ccm-filter"
        />
        <Select<CostType>
          label="Cost Type"
          value={costType}
          options={[
            { value: 'amortized', label: 'Amortized' },
            { value: 'billed', label: 'Billed' },
          ]}
          onChange={setCostType}
          triggerClassName="dd-trigger ccm-filter"
        />
        <label className="ccm-search ccm-tags-search">
          <Search size={15} />
          <input placeholder="Search for tags" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
      </div>
      <div className="ccm-tags">
        <aside className="ccm-tag-sources">
          <button className="ccm-tag-sources-head" onClick={() => setSourcesOpen((o) => !o)}>
            <ChevronDown size={14} className={sourcesOpen ? '' : 'ccm-rot'} /> Tag Sources
          </button>
          {sourcesOpen &&
            SOURCES.map((g) => (
              <div key={g.group}>
                <h5>{g.group}</h5>
                {g.items.map((s) => (
                  <label key={s.id} className="ccm-check">
                    <input
                      type="checkbox"
                      checked={!hidden.has(s.id)}
                      onChange={() =>
                        setHidden((h) => {
                          const n = new Set(h);
                          if (n.has(s.id)) n.delete(s.id);
                          else n.add(s.id);
                          return n;
                        })
                      }
                    />
                    <SourceIcon source={s.id} /> {s.label}
                  </label>
                ))}
              </div>
            ))}
        </aside>
        <div className="ccm-tags-table">
          {tags.isPending ? (
            <Loading label="Loading tags…" />
          ) : tags.isError ? (
            <ErrorState error={tags.error} onRetry={() => tags.refetch()} />
          ) : !rows.length ? (
            <Empty>No tags on the costs of this month.</Empty>
          ) : (
            <table className="ccm-table">
              <thead>
                <tr>
                  <th>TAG NAME</th>
                  <th>SOURCES</th>
                  <th>VALUES</th>
                  <th>COST COVERAGE</th>
                  <th className="num">DISTINCT VALUES</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((t) => {
                  const open = expanded === t.key;
                  const chips = open ? t.values : t.values.slice(0, CHIPS);
                  return (
                    <tr key={t.key}>
                      <td>
                        <Link to={`/cloud-cost/explorer?${new URLSearchParams({ groupBy: t.key, range: '1mo' })}`}>{t.key}</Link>
                      </td>
                      <td title={t.source}>
                        <SourceIcon source={t.source} />
                      </td>
                      <td>
                        <div className="ccm-chips">
                          {chips.map((v) => (
                            <span key={v.value} className="ccm-chip">
                              {v.value}
                            </span>
                          ))}
                          {t.distinctValues > CHIPS && (
                            <button className="ccm-more" onClick={() => setExpanded(open ? null : t.key)}>
                              {open ? 'Less' : `+${t.distinctValues - CHIPS}`}
                            </button>
                          )}
                        </div>
                      </td>
                      <td>
                        <div className="ccm-coverage">
                          <ShareBar share={t.coverage} />
                          <span>{(t.coverage * 100).toFixed(1)}%</span>
                        </div>
                      </td>
                      <td className="num muted-text">
                        {t.distinctValues} ({t.accounts} {t.accounts === 1 ? 'Account' : 'Accounts'})
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

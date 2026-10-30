import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { squarify } from '../metrics/squarify';
import { RANGES, filterQuery, money, pctChange, pctLabel, providerLabel, rangeOf, useCosts, useCostTags, type Filters } from './costModel';
import { CostFilterBar, ProviderIcon } from './widgets';

const W = 1000;
const H = 600;
/** Height of a provider's title, in layout units. */
const HEAD = 26;

const GROUPS = [
  { value: 'servicename', label: 'Service Name' },
  { value: 'subaccountname', label: 'Sub Account Name' },
  { value: 'team', label: 'Team' },
  { value: 'regionname', label: 'Region Name' },
];

interface Tile {
  provider: string;
  name: string;
  cost: number;
  prev: number;
}

/** Red when a tile cost more than before, green when less; stronger the bigger the change. */
function tone(cost: number, prev: number): string {
  const p = pctChange(cost, prev);
  if (p == null) return 'var(--ccm-map-up-3)';
  const step = Math.abs(p) < 2 ? 0 : Math.abs(p) < 10 ? 1 : Math.abs(p) < 30 ? 2 : 3;
  if (step === 0) return 'var(--ccm-map-flat)';
  return `var(--ccm-map-${p > 0 ? 'up' : 'down'}-${step})`;
}

const pct = (v: number, of: number) => `${(v / of) * 100}%`;

/** Summarize → Map: the costs as nested boxes (provider, then a tag), sized by cost and colored by their change. */
export function CostMapPage() {
  const navigate = useNavigate();
  const [rangeKey, setRangeKey] = useState('1mo');
  const [groupBy, setGroupBy] = useState('servicename');
  const [filters, setFilters] = useState<Filters>({});
  const range = useMemo(() => rangeOf(rangeKey), [rangeKey]);
  const tags = useCostTags(range);
  const query = filterQuery(filters);
  const q = useCosts({ query, range, interval: 'month', groupBy: ['providername', groupBy], limit: 200 });
  const layout = useMemo(() => {
    const byProvider = new Map<string, Tile[]>();
    for (const s of q.data?.series ?? []) {
      if (s.tags.other || s.total <= 0) continue;
      const list = byProvider.get(s.tags.providername) ?? [];
      list.push({ provider: s.tags.providername, name: s.tags[groupBy], cost: s.total, prev: s.previousTotal });
      byProvider.set(s.tags.providername, list);
    }
    const providers = [...byProvider].map(([provider, tiles]) => ({ provider, tiles, cost: tiles.reduce((a, t) => a + t.cost, 0), prev: tiles.reduce((a, t) => a + t.prev, 0) }));
    return squarify(providers, (p) => p.cost, W, H).map((box) => ({
      ...box,
      inner: squarify(box.item.tiles, (t) => t.cost, box.w - 4, Math.max(0, box.h - HEAD - 4)),
    }));
  }, [q.data, groupBy]);
  return (
    <div className="ccm-page">
      <CostFilterBar keys={['providername', 'team', 'env']} labels={{ providername: 'Provider', team: 'Team', env: 'Env' }} filters={filters} onChange={setFilters} tags={tags.data}>
        <span className="ccm-filterbar-right">
          <Select label="Group by" value={groupBy} options={GROUPS} onChange={setGroupBy} triggerClassName="dd-trigger ccm-filter" />
          <Select
            label="UTC"
            value={rangeKey}
            options={RANGES.map((r) => ({ value: r.value, label: r.label, icon: <span className="ccm-period">{r.value}</span> }))}
            onChange={setRangeKey}
            align="right"
            triggerClassName="dd-trigger ccm-period-select"
          />
        </span>
      </CostFilterBar>
      <div className="ccm-overview">
        <div className="ccm-map-legend">
          <span>Size: cost over the range</span>
          <span>
            Color: change from the period before
            <i style={{ background: 'var(--ccm-map-down-3)' }} />
            <i style={{ background: 'var(--ccm-map-down-1)' }} />
            <i style={{ background: 'var(--ccm-map-flat)' }} />
            <i style={{ background: 'var(--ccm-map-up-1)' }} />
            <i style={{ background: 'var(--ccm-map-up-3)' }} />
          </span>
        </div>
        {q.isPending ? (
          <Loading label="Loading costs…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : !layout.length ? (
          <Empty>No costs in this range.</Empty>
        ) : (
          <div className="ccm-map">
            {layout.map((p) => (
              <div key={p.item.provider} className="ccm-map-provider" style={{ left: pct(p.x, W), top: pct(p.y, H), width: pct(p.w, W), height: pct(p.h, H) }}>
                <div className="ccm-map-provider-head" style={{ height: pct(HEAD, p.h) }}>
                  <ProviderIcon provider={p.item.provider} size={16} /> <b>{providerLabel(p.item.provider)}</b> <span>{money(p.item.cost)}</span>
                </div>
                {p.inner.map((t) => {
                  const big = t.w > 90 && t.h > 40;
                  const scope = filterQuery({ providername: [t.item.provider], [groupBy]: [t.item.name] }, query);
                  return (
                    <button
                      key={t.item.name}
                      className="ccm-map-tile"
                      title={`${t.item.name}: ${money(t.item.cost, { precise: true })} (${pctLabel(pctChange(t.item.cost, t.item.prev))})`}
                      onClick={() => navigate(`/cloud-cost/explorer?${new URLSearchParams({ q: scope, groupBy: 'servicename,subaccountname', range: rangeKey })}`)}
                      style={{
                        left: pct(t.x + 2, p.w),
                        top: pct(t.y + HEAD + 2, p.h),
                        width: pct(t.w, p.w),
                        height: pct(t.h, p.h),
                        background: tone(t.item.cost, t.item.prev),
                      }}
                    >
                      {big && (
                        <>
                          <b>{t.item.name}</b>
                          <span>{money(t.item.cost)}</span>
                          <small>{pctLabel(pctChange(t.item.cost, t.item.prev))}</small>
                        </>
                      )}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

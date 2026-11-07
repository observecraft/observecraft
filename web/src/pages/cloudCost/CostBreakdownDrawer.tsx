// Cost Explorer → "See More": one group's cost broken down by resource (the bills' ResourceId: which VM, instance,
// bucket or model deployment), by charge (the meter: instance type, input or output tokens), by model, region or
// account, with each row's change against the previous period.
import { useEffect, useMemo, useState } from 'react';
import { Filter, X } from 'lucide-react';
import type { CostType } from '../../api/cloudCost/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { filterQuery, groupLabel, money, pctChange, signedMoney, useCostTags, useCosts } from './costModel';
import { ChangePill, ShareBar, costValueIcon } from './widgets';

/** The breakdowns offered, when the group's costs carry the tag. */
const DIMENSIONS: [key: string, label: string][] = [
  ['resourceid', 'Resource'],
  ['chargedescription', 'Charge'],
  ['ai_model', 'Model'],
  ['servicename', 'Service'],
  ['regionname', 'Region'],
  ['subaccountname', 'Account'],
];

const LIMIT = 50;

/** A resource id's readable name: the last segment of an ARN or an Azure / Google Cloud resource path. */
export function resourceName(id: string): string {
  if (!id || id === 'N/A') return 'Not attributed to a resource';
  return id.split(/[/:]/).filter(Boolean).pop() ?? id;
}

export function CostBreakdownDrawer({
  tags,
  query,
  range,
  costType,
  usageOnly,
  onFilter,
  onClose,
}: {
  /** The group's tags (its row in the explorer). */
  tags: Record<string, string>;
  /** The explorer's own query, which the group narrows. */
  query: string;
  range: { from: number; to: number };
  costType: CostType;
  usageOnly: boolean;
  /** Narrows the explorer to the group instead. */
  onFilter: () => void;
  onClose: () => void;
}) {
  const scope = useMemo(
    () =>
      filterQuery(
        Object.fromEntries(
          Object.entries(tags)
            .filter(([k, v]) => k !== 'other' && v !== 'N/A')
            .map(([k, v]) => [k, [v]]),
        ),
        query,
      ),
    [tags, query],
  );
  const available = useCostTags(range, scope, costType);
  const dims = DIMENSIONS.filter(([k]) => !(k in tags) && (available.data ?? []).some((t) => t.key === k && t.values.some((v) => v.value !== 'N/A')));
  const [dim, setDim] = useState('resourceid');
  const by = dims.some(([k]) => k === dim) ? dim : (dims[0]?.[0] ?? 'resourceid');
  const q = useCosts({ query: scope, range, interval: 'day', groupBy: [by], costType, usageOnly, limit: LIMIT });
  // While another breakdown loads, the previous one (grouped by something else) isn't shown.
  const fresh = (q.data?.series ?? []).every((s) => s.tags.other || by in s.tags);
  const rows = fresh ? (q.data?.series ?? []) : [];
  const total = q.data?.total ?? 0;
  const max = Math.max(1, ...rows.map((s) => Math.abs(s.total)));
  const title = groupLabel(Object.fromEntries(Object.entries(tags).filter(([k]) => k !== 'other'))) || 'All costs';
  const icon = costValueIcon('servicename', tags.servicename ?? '', 22) ?? costValueIcon('providername', tags.providername ?? '', 22);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <>
      <div className="ccm-rec-backdrop" onClick={onClose} />
      <aside className="ccm-rec-drawer ccm-breakdown" aria-label={`Cost of ${title}`}>
        <header className="ccm-rec-drawer-head">
          {icon}
          <div>
            <h2>{title}</h2>
            <span className="muted-text">{q.data ? `${money(total)} in this period` : ' '}</span>
          </div>
          <button className="icon-btn" title="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="ccm-breakdown-bar">
          <Select label="Break down by" value={by} options={dims.map(([value, label]) => ({ value, label }))} onChange={setDim} triggerClassName="dd-trigger ccm-filter" />
          <button className="btn btn-sm" onClick={onFilter} title="Show only this group in the explorer">
            <Filter size={13} /> Filter the explorer
          </button>
        </div>
        <div className="ccm-rec-drawer-body">
          {q.isLoading || !fresh ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : !rows.length ? (
            <p className="muted-text">No costs in this period.</p>
          ) : (
            <table className="ccm-table ccm-breakdown-table">
              <thead>
                <tr>
                  <th>{(dims.find(([k]) => k === by)?.[1] ?? by).toUpperCase()}</th>
                  <th className="num">COST</th>
                  <th className="num">CHANGE</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((s, i) => {
                  const value = s.tags[by] ?? s.tags.other ?? '';
                  const other = !!s.tags.other;
                  return (
                    <tr key={`${i}:${value}`}>
                      <td className="ccm-breakdown-name">
                        {other ? (
                          <span className="muted-text">Other</span>
                        ) : by === 'resourceid' ? (
                          <>
                            <b>{resourceName(value)}</b>
                            {value !== 'N/A' && (
                              <span className="ccm-mono muted-text ccm-breakdown-id" title={value}>
                                {value}
                              </span>
                            )}
                          </>
                        ) : (
                          <span className="ccm-breakdown-value">
                            {costValueIcon(by, value)}
                            {value === 'N/A' ? <span className="muted-text">Not set</span> : value}
                          </span>
                        )}
                      </td>
                      <td className="num ccm-cost-cell">
                        {money(s.total, { precise: true })} <ShareBar share={Math.abs(s.total) / max} />
                      </td>
                      <td className="num">
                        <span className="ccm-breakdown-delta">{signedMoney(s.total - s.previousTotal)}</span>
                        <ChangePill pct={pctChange(s.total, s.previousTotal)} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
          {rows.length >= LIMIT && <p className="muted-text">The {LIMIT} largest, the rest summed as Other.</p>}
        </div>
      </aside>
    </>
  );
}

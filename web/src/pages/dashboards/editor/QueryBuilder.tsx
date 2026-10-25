import { useMemo, useState } from 'react';
import { ChevronDown, Code2 } from 'lucide-react';
import type { SpaceAggregator } from '../../../api/dashboards/types';
import { Select, usePopover } from '../../../components/ui/Dropdown';
import { parseMetricQuery } from '../../../lib/metricQuery';
import { useMetricCatalog } from '../queries';

const AGGS: SpaceAggregator[] = ['avg', 'sum', 'min', 'max'];

function filtersText(q: ReturnType<typeof parseMetricQuery>): string {
  if (!q) return '';
  return [...q.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`), ...q.vars].join(',');
}

function ByPicker({ tags, value, onChange }: { tags: string[]; value: string[]; onChange: (v: string[]) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button type="button" className="dd-trigger qb-by" onClick={() => setOpen((o) => !o)}>
        <span className="dd-trigger-label">{value.length ? value.join(', ') : 'everything'}</span>
        <ChevronDown size={14} className="dd-caret" />
      </button>
      {open && (
        <div className="dd-menu dd-left options-menu-list">
          {tags.map((t) => (
            <label key={t}>
              <input type="checkbox" className="checkbox" checked={value.includes(t)} onChange={() => onChange(value.includes(t) ? value.filter((x) => x !== t) : [...value, t])} />
              {t}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

/** Metric query editor: structured (aggregation · metric · from · by) with a raw-text escape hatch. */
export function QueryBuilder({ value, onChange }: { value: string; onChange: (q: string) => void }) {
  const catalog = useMetricCatalog();
  const parsed = useMemo(() => parseMetricQuery(value), [value]);
  const [raw, setRaw] = useState(!parsed);
  const [from, setFrom] = useState(() => filtersText(parsed));

  const metric = parsed?.metric ?? '';
  const entry = catalog.data?.find((m) => m.name === metric);
  const compose = (p: { agg?: string; metric?: string; from?: string; by?: string[] }) => {
    const agg = p.agg ?? parsed?.aggregator ?? 'avg';
    const m = p.metric ?? metric;
    const f = (p.from ?? from).trim() || '*';
    const by = p.by ?? parsed?.by ?? [];
    onChange(`${agg}:${m}{${f}}${by.length ? ` by {${by.join(',')}}` : ''}`);
  };

  return (
    <div className="qb">
      {raw || !parsed ? (
        <input className="qb-raw mono" value={value} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
      ) : (
        <div className="qb-row">
          <Select value={parsed.aggregator} options={AGGS.map((a) => ({ value: a, label: a }))} onChange={(a) => compose({ agg: a })} triggerClassName="dd-trigger qb-agg" />
          <Select
            value={metric}
            options={(catalog.data ?? []).map((m) => ({ value: m.name, label: m.name, hint: m.unit }))}
            onChange={(m) => compose({ metric: m, by: (parsed.by ?? []).filter((b) => catalog.data?.find((x) => x.name === m)?.tags.includes(b)) })}
            triggerClassName="dd-trigger qb-metric mono"
          />
          <span className="qb-word">from</span>
          <input
            className="qb-from mono"
            placeholder="* or tag:value,$var"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            onBlur={() => compose({ from })}
            onKeyDown={(e) => e.key === 'Enter' && compose({ from })}
          />
          <span className="qb-word">by</span>
          <ByPicker tags={entry?.tags ?? []} value={parsed.by} onChange={(by) => compose({ by })} />
        </div>
      )}
      <button type="button" className={`icon-square qb-toggle${raw ? ' on' : ''}`} title={raw ? 'Structured editor' : 'Edit as text'} onClick={() => setRaw((r) => !r)}>
        <Code2 size={15} />
      </button>
      {!parsed && <div className="qb-error">Expected: avg:metric.name{'{tag:value}'} by {'{tag}'}</div>}
    </div>
  );
}

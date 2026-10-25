import { useState, type ReactNode } from 'react';
import { ChevronDown, ChevronRight, Gauge, Pin, Server, X } from 'lucide-react';
import type { Cloud, CollectorKind, HostOs, MetricFilter } from '../../api/infrastructure/types';
import { Select } from '../../components/ui/Dropdown';
import { clearKey, selectedValues, toggleValue, type Term } from '../../lib/searchQuery';
import { CLOUD_LABEL, COLLECTOR_LABEL, CloudIcon, CollectorIcon, GpuIcon, OS_LABEL, OsIcon } from './hostIcons';

const CLOUDS: Exclude<Cloud, 'none'>[] = ['aws', 'azure', 'gcp', 'oracle', 'alibaba'];
const COLLECTORS: CollectorKind[] = ['observecraft', 'otel'];
const OSES: HostOs[] = ['windows', 'linux', 'darwin'];
const FACET_KEYS = ['cloud', 'collector', 'os', 'gpu'];

export const METRICS = [
  { value: 'system.uptime', label: 'system.uptime', unit: 's' },
  { value: 'system.mem.pct_usable', label: 'system.mem.pct_usable', unit: '%' },
  { value: 'system.cpu.user', label: 'system.cpu.user', unit: '%' },
  { value: 'system.load.1', label: 'system.load.1', unit: '' },
];

interface Props {
  terms: Term[];
  onTerms: (t: Term[]) => void;
  metricFilters: MetricFilter[];
  onMetricFilters: (f: MetricFilter[]) => void;
}

function Row({ checked, onChange, icon, label }: { checked: boolean; onChange: () => void; icon: ReactNode; label: string }) {
  return (
    <label className="hf-row">
      <input type="checkbox" className="checkbox" checked={checked} onChange={onChange} />
      <span className="hf-icon">{icon}</span>
      {label}
    </label>
  );
}

/** Host list sidebar: cloud / collector / OS / GPU checkboxes (edit the query) and metric range filters. */
export function HostFilters({ terms, onTerms, metricFilters, onMetricFilters }: Props) {
  const selected = selectedValues(terms);
  const [teamsOpen, setTeamsOpen] = useState(false);
  const [metricsOpen, setMetricsOpen] = useState(true);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const isOn = (key: string, v: string) => (selected[key] ?? []).includes(v);
  const toggle = (key: string, v: string) => onTerms(toggleValue(terms, key, v));
  const anyFacet = FACET_KEYS.some((k) => (selected[k] ?? []).length > 0);
  const team = selected.team?.[0];

  const updateFilter = (metric: string, patch: Partial<MetricFilter>) =>
    onMetricFilters(metricFilters.map((f) => (f.metric === metric ? { ...f, ...patch } : f)));

  return (
    <aside className="host-filters">
      <div className="et-side-card" onClick={() => setTeamsOpen((o) => !o)}>
        <span className="et-side-toggle">
          {teamsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Teams
        </span>
        <Pin size={14} className="muted" />
      </div>
      {teamsOpen && (
        <ul className="et-team-list">
          {['storefront', 'billing', 'platform', 'payments'].map((t) => (
            <li key={t}>
              <label>
                <input type="radio" name="host-team" checked={team === t} onChange={() => onTerms(toggleValue(clearKey(terms, 'team'), 'team', t))} /> {t}
              </label>
            </li>
          ))}
          <li>
            <button className="link-btn" onClick={() => onTerms(clearKey(terms, 'team'))}>
              All teams
            </button>
          </li>
        </ul>
      )}

      <button className={`hf-all${anyFacet ? '' : ' active'}`} onClick={() => onTerms(FACET_KEYS.reduce((t, k) => clearKey(t, k), terms))}>
        <Server size={16} /> All Hosts
      </button>

      <div className="hf-group">
        {CLOUDS.map((c) => (
          <Row key={c} checked={isOn('cloud', c)} onChange={() => toggle('cloud', c)} icon={<CloudIcon cloud={c} />} label={CLOUD_LABEL[c]} />
        ))}
      </div>
      <div className="hf-group">
        {COLLECTORS.map((c) => (
          <Row key={c} checked={isOn('collector', c)} onChange={() => toggle('collector', c)} icon={<CollectorIcon kind={c} />} label={COLLECTOR_LABEL[c]} />
        ))}
      </div>
      <div className="hf-group">
        {OSES.map((o) => (
          <Row key={o} checked={isOn('os', o)} onChange={() => toggle('os', o)} icon={<OsIcon os={o} />} label={OS_LABEL[o]} />
        ))}
      </div>
      <div className="hf-group">
        <Row checked={isOn('gpu', 'true')} onChange={() => toggle('gpu', 'true')} icon={<GpuIcon />} label="GPU" />
      </div>

      <div className="hf-metrics">
        <button className="hf-metrics-head" onClick={() => setMetricsOpen((o) => !o)}>
          {metricsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <Gauge size={15} /> Filter Metrics
        </button>
        {metricsOpen && (
          <>
            <Select
              value=""
              options={[{ value: '', label: 'select a metric' }, ...METRICS.filter((m) => !metricFilters.some((f) => f.metric === m.value))]}
              onChange={(m) => m && onMetricFilters([...metricFilters, { metric: m }])}
              triggerClassName="dd-trigger hf-metric-select"
            />
            {metricFilters.map((f) => {
              const open = expanded.has(f.metric);
              const unit = METRICS.find((m) => m.value === f.metric)?.unit ?? '';
              return (
                <div key={f.metric} className="hf-metric">
                  <div className="hf-metric-head">
                    <button
                      onClick={() =>
                        setExpanded((s) => {
                          const n = new Set(s);
                          if (n.has(f.metric)) n.delete(f.metric);
                          else n.add(f.metric);
                          return n;
                        })
                      }
                    >
                      {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      <strong>{f.metric}</strong>
                    </button>
                    <button className="icon-plain" title="Remove filter" onClick={() => onMetricFilters(metricFilters.filter((x) => x.metric !== f.metric))}>
                      <X size={13} />
                    </button>
                  </div>
                  {open && (
                    <div className="hf-range">
                      <input
                        type="number"
                        placeholder="min"
                        value={f.min ?? ''}
                        onChange={(e) => updateFilter(f.metric, { min: e.target.value === '' ? undefined : Number(e.target.value) })}
                      />
                      <span>–</span>
                      <input
                        type="number"
                        placeholder="max"
                        value={f.max ?? ''}
                        onChange={(e) => updateFilter(f.metric, { max: e.target.value === '' ? undefined : Number(e.target.value) })}
                      />
                      <span className="muted-text">{unit}</span>
                    </div>
                  )}
                </div>
              );
            })}
          </>
        )}
      </div>
    </aside>
  );
}

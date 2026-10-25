import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Search } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useMetricCatalog } from '../dashboards/queries';
import { KIND_LABEL } from './MetricsLayout';

/** Metrics → Summary: every metric reported in the last day, with its type, unit and tags. */
export function MetricsSummary() {
  const q = useMetricCatalog();
  const [text, setText] = useState('');
  const [kinds, setKinds] = useState<string[]>([]);
  if (q.isPending) return <Loading label="Loading metrics…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const all = q.data;
  const counts = all.reduce<Record<string, number>>((acc, m) => ((acc[m.kind ?? 'gauge'] = (acc[m.kind ?? 'gauge'] ?? 0) + 1), acc), {});
  const t = text.trim().toLowerCase();
  const rows = all.filter((m) => (!t || m.name.toLowerCase().includes(t) || m.tags.some((g) => g.toLowerCase().includes(t))) && (kinds.length === 0 || kinds.includes(m.kind ?? 'gauge')));
  return (
    <div className="mx-page">
      <div className="section-header">
        <div className="section-header-left">
          <label className="tv-filter">
            <Search size={15} />
            <input placeholder="Search metric names or tags" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
          <div className="mx-kind-filters">
            {Object.entries(counts).map(([k, n]) => (
              <button key={k} className={`mx-kind-chip${kinds.includes(k) ? ' active' : ''}`} onClick={() => setKinds((ks) => (ks.includes(k) ? ks.filter((x) => x !== k) : [...ks, k]))}>
                {KIND_LABEL[k] ?? k} <span className="muted">{n}</span>
              </button>
            ))}
          </div>
        </div>
        <span className="muted">
          {rows.length} of {all.length} metrics
        </span>
      </div>
      <div className="sx-results">
        <table className="sx-table mx-summary">
          <thead>
            <tr>
              <th>METRIC</th>
              <th>TYPE</th>
              <th>UNIT</th>
              <th>TAGS</th>
              <th>DESCRIPTION</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => (
              <tr key={m.name}>
                <td className="mono">
                  <Link to={`/metrics?metric=${encodeURIComponent(m.name)}`}>{m.name}</Link>
                </td>
                <td>
                  <span className="mx-kind">{KIND_LABEL[m.kind ?? 'gauge'] ?? m.kind}</span>
                </td>
                <td className="muted">{m.unit ?? '—'}</td>
                <td className="ellipsis mx-tags" title={m.tags.join(', ')}>
                  {m.tags.slice(0, 6).join(', ')}
                  {m.tags.length > 6 && <span className="more-chip">+{m.tags.length - 6}</span>}
                </td>
                <td className="muted ellipsis">{m.description}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

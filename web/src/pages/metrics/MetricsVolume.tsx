import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { Search } from 'lucide-react';
import { metricVolume } from '../../api/dashboards/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useTime } from '../../components/shell/TimeContext';
import { KIND_LABEL } from './MetricsLayout';

/** Metrics → Volume: metrics by cardinality (distinct series in the range), the biggest first. */
export function MetricsVolume() {
  const { range } = useTime();
  const q = useQuery({ queryKey: ['metrics', 'volume', range], queryFn: ({ signal }) => metricVolume(range, signal) });
  const [text, setText] = useState('');
  if (q.isPending) return <Loading label="Counting series…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const t = text.trim().toLowerCase();
  const rows = q.data.metrics.filter((m) => !t || m.name.toLowerCase().includes(t));
  // Bars relative to the biggest metric, so the ranking reads at a glance.
  const top = Math.max(1, ...q.data.metrics.map((m) => m.series));
  return (
    <div className="mx-page">
      <div className="section-header">
        <div className="section-header-left">
          <label className="tv-filter">
            <Search size={15} />
            <input placeholder="Search metric names" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
        </div>
        <span className="muted">
          {q.data.totalSeries.toLocaleString()} series across {q.data.metrics.length.toLocaleString()} metrics
        </span>
      </div>
      <div className="sx-results">
        <table className="sx-table mx-volume">
          <thead>
            <tr>
              <th>METRIC</th>
              <th>TYPE</th>
              <th className="num">ACTIVE SERIES</th>
              <th>SHARE OF ALL SERIES</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => (
              <tr key={m.name}>
                <td className="mono">
                  <Link to={`/metrics?metric=${encodeURIComponent(m.name)}`}>{m.name}</Link>
                </td>
                <td>
                  <span className="mx-kind">{KIND_LABEL[m.kind] ?? m.kind}</span>
                </td>
                <td className="num">{m.series.toLocaleString()}</td>
                <td>
                  <span className="mx-share">
                    <span className="mx-share-bar" style={{ width: `${Math.max(1, (m.series / top) * 100)}%` }} />
                  </span>
                  <span className="muted">{(m.share * 100).toFixed(m.share < 0.01 ? 2 : 1)}%</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

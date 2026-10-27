import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { logPatterns, type LogPattern } from '../../api/client';
import type { LogStatus, TimeRange } from '../../api/types';
import { STATUS_COLORS } from '../../theme/chartTheme';

/** Group into Patterns: the matching logs grouped by message pattern, most frequent first. */
export function PatternsTable({ range, query, onOpen }: { range: TimeRange; query: string; onOpen: (p: LogPattern) => void }) {
  const result = useQuery({ queryKey: ['logs', 'patterns', range, query], queryFn: () => logPatterns({ range, query }), placeholderData: keepPreviousData });
  const patterns = result.data ?? [];
  const total = patterns.reduce((n, p) => n + p.count, 0);
  return (
    <table className="log-table log-patterns">
      <thead>
        <tr>
          <th className="col-status" />
          <th className="col-count">COUNT</th>
          <th className="col-share">%</th>
          <th className="col-service">SERVICE</th>
          <th>PATTERN</th>
        </tr>
      </thead>
      <tbody>
        {patterns.map((p) => (
          <tr key={p.pattern} onClick={() => onOpen(p)} title={p.sample}>
            <td className="col-status">
              <span className="status-bar" style={{ background: `var(${STATUS_COLORS[p.status as LogStatus] ?? STATUS_COLORS.info})` }} />
            </td>
            <td className="mono col-count">{p.count.toLocaleString()}</td>
            <td className="mono col-share">{total ? `${((p.count / total) * 100).toFixed(1)}%` : '—'}</td>
            <td className="mono">{p.service}</td>
            <td className="mono col-content">
              {p.pattern.split('*').map((part, i) => (
                <span key={i}>
                  {i > 0 && <span className="pattern-wildcard">*</span>}
                  {part}
                </span>
              ))}
            </td>
          </tr>
        ))}
        {!result.isPending && !patterns.length && (
          <tr>
            <td colSpan={5} className="log-empty">
              {result.error ? `Patterns could not be loaded: ${result.error.message}` : 'No logs match this search.'}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

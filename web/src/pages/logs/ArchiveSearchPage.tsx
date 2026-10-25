import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, Search, Trash2, X } from 'lucide-react';
import { archiveSearch, deleteArchiveSearch, listArchiveSearches, startArchiveSearch } from '../../api/archives/client';
import type { LogEvent } from '../../api/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { formatRange, formatSize, RangeFields, useArchives } from './archiveBits';
import { LogDetail } from './LogDetail';
import { LogTable } from './LogTable';

const HOUR = 3_600_000;

/** Logs → Archive Search: queries archived Parquet files (DuckDB) without rehydrating them. */
export function ArchiveSearchPage() {
  const qc = useQueryClient();
  const archives = useArchives();
  const [archiveId, setArchiveId] = useState('');
  const [query, setQuery] = useState('');
  const [range, setRange] = useState(() => ({ from: Date.now() - 24 * HOUR, to: Date.now() }));
  const [active, setActive] = useState<string | null>(null);
  const [selected, setSelected] = useState<LogEvent | null>(null);
  const history = useQuery({ queryKey: ['archive-searches'], queryFn: listArchiveSearches, refetchInterval: 5000 });
  const start = useMutation({
    mutationFn: () => startArchiveSearch(archiveId || archives.data![0].id, query, range),
    onSuccess: (s) => {
      setActive(s.id);
      void qc.invalidateQueries({ queryKey: ['archive-searches'] });
    },
  });
  const remove = useMutation({ mutationFn: deleteArchiveSearch, onSuccess: () => qc.invalidateQueries({ queryKey: ['archive-searches'] }) });
  const detail = useQuery({
    queryKey: ['archive-search', active],
    queryFn: () => archiveSearch(active!),
    enabled: !!active,
    refetchInterval: (q) => (q.state.data?.search.status === 'running' ? 1000 : false),
  });
  useEffect(() => setSelected(null), [active]);

  if (archives.isPending) return <Loading label="Loading archives…" />;
  if (archives.isError) return <ErrorState error={archives.error} onRetry={() => archives.refetch()} />;
  if (archives.data.length === 0)
    return (
      <div className="ar-page">
        <section className="settings-card settings-empty">
          <Archive size={28} />
          <h2>No archives yet</h2>
          <p className="muted">Archives keep your logs in your own S3-compatible bucket after they leave the live index. Set one up, then search it here.</p>
          <Link className="btn btn-primary" to="/logs/archives">
            Set up an archive
          </Link>
        </section>
      </div>
    );

  const s = detail.data?.search;
  const pct = s && s.filesTotal > 0 ? Math.round((s.filesScanned / s.filesTotal) * 100) : 0;
  return (
    <div className="ar-page">
      <form
        className="ar-search-form"
        onSubmit={(e) => {
          e.preventDefault();
          start.mutate();
        }}
      >
        <label className="form-label">
          Archive
          <select value={archiveId || archives.data[0].id} onChange={(e) => setArchiveId(e.target.value)}>
            {archives.data.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name} ({a.bucket}/{a.prefix})
              </option>
            ))}
          </select>
        </label>
        <label className="form-label ar-query">
          Query
          <span className="ar-query-input">
            <Search size={15} />
            <input placeholder='service:order-service status:error "bad response"' value={query} onChange={(e) => setQuery(e.target.value)} />
          </span>
        </label>
        <RangeFields range={range} onChange={setRange} />
        <button className="btn btn-primary" disabled={start.isPending || range.to <= range.from}>
          Search Archive
        </button>
      </form>
      {start.error && <p className="form-error">{(start.error as Error).message}</p>}

      {s && (
        <section className="ar-result">
          <div className="ar-result-head">
            <div>
              <b>{s.status === 'running' ? 'Searching…' : s.status === 'done' ? `${s.matches.toLocaleString()} matching logs` : s.status === 'canceled' ? 'Canceled' : 'Search failed'}</b>{' '}
              <span className="muted">
                {s.archiveName} · {formatRange(s.from, s.to)} · {s.query || 'all logs'}
              </span>
            </div>
            <div className="ar-progress">
              <span className="muted">
                {s.filesScanned}/{s.filesTotal} files · {formatSize(s.bytesScanned)}
              </span>
              {s.status === 'running' && (
                <>
                  <span className="ar-bar">
                    <i style={{ width: `${pct}%` }} />
                  </span>
                  <button className="btn btn-sm" onClick={() => remove.mutate(s.id)}>
                    Cancel
                  </button>
                </>
              )}
              <button className="icon-plain" title="Close" onClick={() => setActive(null)}>
                <X size={16} />
              </button>
            </div>
          </div>
          {s.error && <p className="form-error">{s.error}</p>}
          {s.status === 'done' && s.matches > s.maxResults && (
            <p className="muted">Showing the newest {s.maxResults.toLocaleString()}. Narrow the query or range to see the rest, or rehydrate them.</p>
          )}
          {s.status === 'done' && s.filesTotal === 0 && <p className="muted">Nothing was archived in this time range.</p>}
          {detail.data && detail.data.events.length > 0 && (
            <div className="ar-table">
              <LogTable logs={detail.data.events as LogEvent[]} selectedId={selected?.id ?? null} onSelect={setSelected} />
            </div>
          )}
        </section>
      )}

      <h2 className="settings-subtitle">Recent searches</h2>
      {history.data && history.data.length === 0 && <p className="muted">No archive searches yet.</p>}
      {history.data && history.data.length > 0 && (
        <table className="settings-table">
          <thead>
            <tr>
              <th>QUERY</th>
              <th>ARCHIVE</th>
              <th>TIME RANGE</th>
              <th>STATUS</th>
              <th className="num">MATCHES</th>
              <th>STARTED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {history.data.map((h) => (
              <tr key={h.id} className={h.id === active ? 'row-active' : ''}>
                <td>
                  <button className="link-btn" onClick={() => setActive(h.id)}>
                    {h.query || 'all logs'}
                  </button>
                </td>
                <td>{h.archiveName}</td>
                <td className="muted">{formatRange(h.from, h.to)}</td>
                <td>{h.status}</td>
                <td className="num">{h.matches.toLocaleString()}</td>
                <td className="muted">{timeAgo(h.createdAt)}</td>
                <td className="num">
                  <button className="icon-plain" title="Delete" onClick={() => remove.mutate(h.id)}>
                    <Trash2 size={15} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {selected && <LogDetail log={selected} onClose={() => setSelected(null)} onFilter={(k, v) => setQuery((q) => `${q} ${k}:${v}`.trim())} />}
    </div>
  );
}

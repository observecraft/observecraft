import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { BellPlus, Search, X } from 'lucide-react';
import { listServiceChecks, serviceCheckDetail, type ServiceCheckGroup } from '../../api/serviceChecks/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatLongDate, timeAgo } from '../../lib/format';
import { checkMonitorLink, filterChecks, STATUS_LABELS, statusClass, statusLabel } from '../../lib/serviceChecks';
import '../infrastructure/infrastructure.css';
import './checks.css';

type SortKey = 'check' | 'name';
const DAY_MS = 86_400_000;

/** Monitoring → Check Summary: every service check reporting in the past day; a check opens its status per host. */
export function CheckSummary() {
  const [params, setParams] = useSearchParams();
  const selected = params.get('check');
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'check', dir: 'asc' });
  const q = useQuery({ queryKey: ['service-checks'], queryFn: ({ signal }) => listServiceChecks(signal) });
  const rows = useMemo(() => filterChecks([...(q.data ?? [])], search, sort.key, sort.dir), [q.data, search, sort]);
  const open = (check: string | null) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (check) next.set('check', check);
      else next.delete('check');
      return next;
    });
  const th = (label: string, key: SortKey) => (
    <th
      className="sortable"
      onClick={() => setSort((s) => ({ key, dir: s.key === key && s.dir === 'asc' ? 'desc' : 'asc' }))}
      aria-sort={sort.key === key ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none'}
    >
      {label}
      {sort.key === key && (sort.dir === 'asc' ? ' ↑' : ' ↓')}
    </th>
  );
  return (
    <div className="cs-page">
      <header className="cs-head">
        <h1 className="cs-title">Check summary</h1>
        <p className="cs-subtitle">All checks reporting across your infrastructure in the past day</p>
      </header>
      <div className="cs-bar">
        <span className="cs-count">{q.data ? (rows.length === 0 ? 'No checks' : `Showing 1–${rows.length} of ${rows.length} checks`) : ' '}</span>
        <label className="tv-filter cs-search">
          <Search size={15} />
          <input placeholder="Search checks" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search checks" />
        </label>
      </div>
      {q.isPending ? (
        <Loading label="Loading checks…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="sx-results">
          <table className="sx-table cs-table">
            <thead>
              <tr>
                {th('CHECK', 'check')}
                {th('NAME', 'name')}
              </tr>
            </thead>
            <tbody>
              {rows.map((c) => (
                <tr key={c.check} className={selected === c.check ? 'selected' : undefined} onClick={() => open(c.check)}>
                  <td className="mono">{c.check}</td>
                  <td>{c.name}</td>
                </tr>
              ))}
              {rows.length === 0 && (
                <tr>
                  <td colSpan={2} className="cs-empty">
                    {q.data.length === 0 ? 'No service checks reported in the past day. The Datadog Agent sends its checks once its API key and site point here.' : 'No check matches the search.'}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
      {selected && <CheckPanel check={selected} onClose={() => open(null)} />}
    </div>
  );
}

/** One check: status counts, its status over the past day per host and tag set, its latest message. */
function CheckPanel({ check, onClose }: { check: string; onClose: () => void }) {
  const [to] = useState(() => Date.now());
  const range = { from: to - DAY_MS, to };
  const q = useQuery({ queryKey: ['service-check', check, to], queryFn: ({ signal }) => serviceCheckDetail(check, range, signal) });
  const d = q.data;
  return (
    <aside className="rc-panel cs-panel" role="dialog" aria-label={`Service check ${check}`}>
      <header>
        <span>Service check</span>
        <button className="rc-panel-close icon-plain" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <h2 className="mono">{check}</h2>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        d && (
          <>
            <div className="rc-panel-actions">
              <span className="muted">{d.name} · past day</span>
              <Link className="btn btn-primary cs-monitor" to={checkMonitorLink(check)}>
                <BellPlus size={14} /> Create monitor
              </Link>
            </div>
            <div className="cs-counts">
              {STATUS_LABELS.map((label, s) => (
                <div key={label} className={`cs-count-tile cs-${statusClass(s)}`}>
                  <span className="cs-count-n">{[d.counts.ok, d.counts.warning, d.counts.critical, d.counts.unknown][s]}</span>
                  <span className="cs-count-label">{label}</span>
                </div>
              ))}
            </div>
            <h3>Latest message</h3>
            {d.latestMessage ? (
              <div className="cs-message">
                <span className={`cs-status cs-${statusClass(d.latestMessage.status)}`}>{statusLabel(d.latestMessage.status)}</span>
                <span className="mono">{d.latestMessage.host}</span>
                <span className="muted" title={formatLongDate(d.latestMessage.at)}>
                  {timeAgo(d.latestMessage.at)}
                </span>
                <p>{d.latestMessage.message}</p>
              </div>
            ) : (
              <p className="muted">No message: the check's runs sent none.</p>
            )}
            <h3>Status over the past day</h3>
            <div className="cs-strip-axis muted">
              <span>{new Date(d.buckets[0] ?? range.from).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
              <span>now</span>
            </div>
            <div className="cs-groups">
              {d.groups.map((g) => (
                <GroupRow key={`${g.host}\u0000${g.tags.join(',')}`} group={g} buckets={d.buckets} bucketMs={d.bucketMs} />
              ))}
              {d.groups.length === 0 && <p className="muted">No runs in the past day.</p>}
            </div>
          </>
        )
      )}
    </aside>
  );
}

function GroupRow({ group, buckets, bucketMs }: { group: ServiceCheckGroup; buckets: number[]; bucketMs: number }) {
  return (
    <div className="cs-group">
      <div className="cs-group-head">
        <span className={`cs-status cs-${statusClass(group.status)}`}>{statusLabel(group.status)}</span>
        <span className="mono cs-host">{group.host || 'no host'}</span>
        {group.tags.map((t) => (
          <span key={t} className="tag-chip">
            {t}
          </span>
        ))}
      </div>
      <div className="cs-strip" role="img" aria-label={`${group.host} status over the past day`}>
        {group.strip.map((s, i) => (
          <span
            key={buckets[i] ?? i}
            className={`cs-cell cs-${statusClass(s)}`}
            title={`${new Date(buckets[i]).toLocaleString()} – ${new Date(buckets[i] + bucketMs).toLocaleTimeString()}: ${s === null || s === undefined ? 'no run' : statusLabel(s)}`}
          />
        ))}
      </div>
      {group.message && <p className="cs-group-message muted">{group.message}</p>}
    </div>
  );
}

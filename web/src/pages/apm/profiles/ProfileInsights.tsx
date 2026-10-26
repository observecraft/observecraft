import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { ArrowUpNarrowWide } from 'lucide-react';
import { profileInsights, setInsightState, type InsightStatus } from '../../../api/profiles/client';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { LanguageMark } from './ProfileExplorer';

const STATUSES: { key: InsightStatus; label: string }[] = [
  { key: 'for_review', label: 'For Review' },
  { key: 'reviewed', label: 'Reviewed' },
  { key: 'resolved', label: 'Resolved' },
  { key: 'ignored', label: 'Ignored' },
];

/** APM → Profiles → Insights: patterns detected in the profiles, with their review state. */
export function ProfileInsights() {
  const { range } = useTime();
  const qc = useQueryClient();
  const [params] = useSearchParams();
  const [env, setEnv] = useState('');
  const [service, setService] = useState(params.get('service') ?? '');
  const [category, setCategory] = useState('');
  const [tab, setTab] = useState<InsightStatus | 'all'>('all');
  const [perPage, setPerPage] = useState(5);
  const [page, setPage] = useState(0);
  const [desc, setDesc] = useState(true);
  const all = useQuery({ queryKey: ['profiles', 'insights', range], queryFn: () => profileInsights({ range }) });
  const update = useMutation({
    mutationFn: ({ key, status }: { key: string; status: InsightStatus }) => setInsightState(key, status),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['profiles', 'insights'] }),
  });

  const items = all.data?.items ?? [];
  const envs = [...new Set(items.map((i) => i.env))].sort();
  const services = [...new Set(items.map((i) => i.service))].sort();
  const categories = [...new Set(items.map((i) => i.category))].sort();
  const scoped = useMemo(() => items.filter((i) => (!env || i.env === env) && (!service || i.service === service) && (!category || i.category === category)), [items, env, service, category]);
  const count = (s: InsightStatus | 'all') => (s === 'all' ? scoped.length : scoped.filter((i) => i.status === s).length);
  const shown = scoped.filter((i) => tab === 'all' || i.status === tab).sort((a, b) => (desc ? b.detectedAt - a.detectedAt : a.detectedAt - b.detectedAt));
  const pageItems = shown.slice(page * perPage, (page + 1) * perPage);

  const select = (label: string, value: string, set: (v: string) => void, options: string[]) => (
    <label className="pf-field">
      <span>{label}</span>
      <select
        value={value}
        onChange={(e) => {
          set(e.target.value);
          setPage(0);
        }}
      >
        <option value="">All</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </label>
  );

  return (
    <div className="pf-insights">
      <div className="pf-insights-filters">
        {select('Env', env, setEnv, envs)}
        {select('Service', service, setService, services)}
        {select('Insight Category', category, setCategory, categories)}
      </div>
      <nav className="pf-status-tabs">
        {[...STATUSES, { key: 'all' as const, label: 'All' }].map((s) => (
          <button
            key={s.key}
            className={`pf-status-tab${tab === s.key ? ' active' : ''}`}
            onClick={() => {
              setTab(s.key);
              setPage(0);
            }}
          >
            {s.label} <span className="pf-count">{count(s.key)}</span>
          </button>
        ))}
      </nav>
      {all.isPending ? (
        <Loading label="Detecting insights…" />
      ) : all.isError ? (
        <ErrorState error={all.error} onRetry={() => all.refetch()} />
      ) : (
        <>
          <table className="pf-table pf-insights-table">
            <thead>
              <tr>
                <th>
                  <button className="pf-sort" onClick={() => setDesc((d) => !d)} title="Sort by detection time">
                    <ArrowUpNarrowWide size={15} className={desc ? '' : 'flip'} /> INSIGHT
                  </button>
                </th>
                <th className="num">STATUS</th>
              </tr>
            </thead>
            <tbody>
              {pageItems.map((i) => (
                <tr key={i.key}>
                  <td>
                    <div className="pf-insight-title">
                      <b>{i.title}</b>
                      <span className={`pf-sev pf-sev-${i.severity.toLowerCase()}`}>{i.severity}</span>
                      <span className="pf-cat">{i.category}</span>
                    </div>
                    <div className="pf-insight-desc">
                      <LanguageMark language={i.language} size={16} /> <span className="pf-dot">•</span> {i.service}: {i.description}
                    </div>
                  </td>
                  <td className="num">
                    <select className={`pf-status pf-status-${i.status}`} value={i.status} onChange={(e) => update.mutate({ key: i.key, status: e.target.value as InsightStatus })}>
                      {STATUSES.map((s) => (
                        <option key={s.key} value={s.key}>
                          {s.label.toUpperCase()}
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              ))}
              {pageItems.length === 0 && (
                <tr>
                  <td colSpan={2} className="pf-empty">
                    No insights in the selected time range.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
          <div className="pf-pager">
            Results per page:
            <select
              value={perPage}
              onChange={(e) => {
                setPerPage(Number(e.target.value));
                setPage(0);
              }}
            >
              {[5, 10, 25, 50].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
            {shown.length > perPage && (
              <span className="btn-group">
                <button className="btn" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
                  Previous
                </button>
                <button className="btn" disabled={(page + 1) * perPage >= shown.length} onClick={() => setPage((p) => p + 1)}>
                  Next
                </button>
              </span>
            )}
          </div>
        </>
      )}
    </div>
  );
}

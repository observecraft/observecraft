import { useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Search } from 'lucide-react';
import { auditTrail } from '../../api/auth/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { formatLogDate } from '../../lib/format';

const PAGE = 50;

/** Security events: sign-ins and failures, MFA and key changes, user and SSO administration. */
export function AuditTrailPage() {
  const [query, setQuery] = useState('');
  const [offset, setOffset] = useState(0);
  const [actors, setActors] = useState<string[]>([]);
  const [categories, setCategories] = useState<string[]>([]);
  const q = useQuery({ queryKey: ['auth', 'audit', query, offset, actors, categories], queryFn: () => auditTrail(query, offset, PAGE, actors, categories), placeholderData: keepPreviousData });
  const toggle = (list: string[], set: (v: string[]) => void, value: string) => {
    set(list.includes(value) ? list.filter((v) => v !== value) : [...list, value]);
    setOffset(0);
  };
  return (
    <div className="settings-pad">
      <h1 className="settings-title">Audit Trail</h1>
      <label className="settings-search wide">
        <Search size={16} />
        <input
          placeholder="Filter by user, action (login.failed, user.invited, …) or target"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOffset(0);
          }}
        />
      </label>
      <div className="audit-body">
        <aside className="audit-facets">
          <AuditFacet title="Category" values={q.data?.categories ?? []} selected={categories} onToggle={(v) => toggle(categories, setCategories, v)} />
          <AuditFacet title="Actor" values={q.data?.actors ?? []} selected={actors} onToggle={(v) => toggle(actors, setActors, v)} />
        </aside>
        <div className="audit-main">
          {q.isPending ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <>
              <div className="results-bar">
                <span>
                  <b>{q.data.total}</b> events
                </span>
              </div>
              <table className="settings-table bordered audit-table">
                <thead>
                  <tr>
                    <th>DATE</th>
                    <th>USER</th>
                    <th>ACTION</th>
                    <th>TARGET</th>
                    <th>DETAIL</th>
                    <th>IP</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.events.map((e) => (
                    <tr key={e.id}>
                      <td className="mono muted">{formatLogDate(e.createdAt)}</td>
                      <td>{e.actor}</td>
                      <td>
                        <span className={`audit-action${e.action.includes('failed') || e.action.includes('locked') ? ' bad' : ''}`}>{e.action}</span>
                      </td>
                      <td className="ellipsis audit-cell">{e.target}</td>
                      <td className="ellipsis audit-cell muted" title={e.detail}>
                        {e.detail}
                      </td>
                      <td className="mono muted">{e.ip ?? ''}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function AuditFacet({ title, values, selected, onToggle }: { title: string; values: { value: string; count: number }[]; selected: string[]; onToggle: (value: string) => void }) {
  return (
    <div className="audit-facet">
      <div className="audit-facet-title">{title}</div>
      {values.length === 0 && <span className="muted">None</span>}
      {values.map((v) => (
        <label key={v.value} className="audit-facet-row">
          <input type="checkbox" className="checkbox" checked={selected.includes(v.value)} onChange={() => onToggle(v.value)} />
          <span className="audit-facet-value">{v.value}</span>
          <span className="muted">{v.count.toLocaleString()}</span>
        </label>
      ))}
    </div>
  );
}

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
  const q = useQuery({ queryKey: ['auth', 'audit', query, offset], queryFn: () => auditTrail(query, offset, PAGE), placeholderData: keepPreviousData });
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
  );
}

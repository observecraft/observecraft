import { useDeferredValue, useState } from 'react';
import { Link } from 'react-router-dom';
import { PhoneCall, Search } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { NewPageModal } from './PagesPage';
import { People, Segmented, errorText } from './common';
import { useOnCallPolicies, useOnCallTeams, useSetOnCallRouting } from './queries';

/** On-Call → Teams: each team's escalation policy (routing), schedules and who's on call now. */
export function OnCallTeamsPage() {
  const q = useOnCallTeams();
  const policies = useOnCallPolicies();
  const route = useSetOnCallRouting();
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);
  const [scope, setScope] = useState<'mine' | 'all'>('all');
  const [paging, setPaging] = useState<string | null>(null);
  const text = deferred.trim().toLowerCase();
  const rows = (q.data ?? []).filter((t) => (scope === 'all' || t.mine) && (!text || t.name.toLowerCase().includes(text) || t.handle.includes(text)));
  const policyOptions = [{ value: '', label: 'None: page every member' }, ...(policies.data ?? []).map((p) => ({ value: p.id, label: p.name }))];
  return (
    <div className="onc-list-page">
      <div className="onc-search-row">
        <div className="mon-filter onc-search">
          <Search size={16} />
          <input placeholder="Search teams" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
      </div>
      <div className="onc-main-bar">
        <Segmented
          value={scope}
          onChange={setScope}
          options={[
            ['mine', 'My Teams'],
            ['all', 'All Teams'],
          ]}
        />
        <span className="muted onc-showing">
          Pages sent to a team follow its escalation policy. Manage members under <Link to="/teams">Teams</Link>.
        </span>
      </div>
      {route.error && <p className="form-error">{errorText(route.error)}</p>}
      {q.isPending ? (
        <Loading label="Loading teams…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="onc-table-wrap">
          <table className="onc-table onc-teams-table">
            <thead>
              <tr>
                <th>TEAM</th>
                <th className="onc-col-routing">ESCALATION POLICY</th>
                <th>SCHEDULES</th>
                <th className="onc-col-resp">ON CALL NOW</th>
                <th className="onc-col-num">OPEN PAGES</th>
                <th className="onc-col-action" />
              </tr>
            </thead>
            <tbody>
              {rows.map((t) => (
                <tr key={t.handle} className="static">
                  <td>
                    <b>{t.name}</b>
                    <span className="onc-tags">
                      @{t.handle} · {t.memberCount} member{t.memberCount === 1 ? '' : 's'}
                    </span>
                  </td>
                  <td className="onc-col-routing">
                    <Select value={t.policy?.id ?? ''} onChange={(v) => route.mutate({ handle: t.handle, policyId: v || null })} options={policyOptions} />
                  </td>
                  <td>
                    {t.schedules.length ? (
                      t.schedules.map((s, i) => (
                        <span key={s.id}>
                          {i > 0 && ', '}
                          <Link to={`/on-call/schedules/${s.id}`}>{s.name}</Link>
                        </span>
                      ))
                    ) : (
                      <span className="muted">—</span>
                    )}
                  </td>
                  <td className="onc-col-resp">
                    <People users={t.onCallNow} empty={t.schedules.length ? 'Nobody' : '—'} />
                  </td>
                  <td className="onc-col-num">{t.openPages ? <Link to={`/on-call/pages?team=${encodeURIComponent(t.handle)}`}>{t.openPages}</Link> : <span className="muted">0</span>}</td>
                  <td className="onc-col-action">
                    <button className="btn btn-sm" onClick={() => setPaging(t.handle)}>
                      <PhoneCall size={13} /> Page
                    </button>
                  </td>
                </tr>
              ))}
              {!rows.length && (
                <tr className="onc-empty-row">
                  <td colSpan={6}>
                    <p className="muted onc-pad">No teams match.</p>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
      {paging && <NewPageModal team={paging} onClose={() => setPaging(null)} />}
    </div>
  );
}

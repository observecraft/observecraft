import { useDeferredValue, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { CalendarDays, Plus, Search, Upload } from 'lucide-react';
import { DAY } from '../../api/onCall/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useProfile } from '../settings/queries';
import { People, Segmented, Showing, TeamFacet, downloadShifts } from './common';
import { useMyOnCallShifts, useOnCallSchedules, useOnCallTeams } from './queries';

/** On-Call → Schedules: every schedule with who is on call now. */
export function SchedulesPage() {
  const navigate = useNavigate();
  const q = useOnCallSchedules();
  const teams = useOnCallTeams();
  const me = useProfile();
  const myId = me.data?.id ?? '';
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);
  const [scope, setScope] = useState<'mine' | 'all'>('mine');
  const [selected, setSelected] = useState<string[]>([]);
  const [facetOpen, setFacetOpen] = useState(false);
  const minute = Math.floor(Date.now() / 60_000) * 60_000;
  const upcomingRange = useMemo(() => ({ from: minute, to: minute + 31 * DAY }), [minute]);
  const upcoming = useMyOnCallShifts(upcomingRange);
  const all = q.data ?? [];
  const text = deferred.trim().toLowerCase();
  const rows = all.filter(
    (s) =>
      (!text || s.name.toLowerCase().includes(text) || (s.team ?? '').includes(text) || s.tags.some((t) => t.includes(text))) &&
      (!selected.length || (s.team != null && selected.includes(s.team))) &&
      (scope === 'all' || s.layers.some((l) => l.members.includes(myId))),
  );
  const counts: Record<string, number> = {};
  for (const s of all) if (s.team) counts[s.team] = (counts[s.team] ?? 0) + 1;
  const filtered = !!text || selected.length > 0 || scope === 'mine';
  const clear = () => {
    setQuery('');
    setSelected([]);
    setScope('all');
  };
  const facet = (
    <TeamFacet teams={(teams.data ?? []).map((t) => ({ handle: t.handle, name: t.name }))} selected={selected} onChange={setSelected} counts={counts} open={facetOpen} onOpenChange={setFacetOpen} />
  );
  return (
    <div className="onc-list-page">
      <div className="onc-search-row">
        <div className="mon-filter onc-search">
          <Search size={16} />
          <input placeholder="Search schedules by name, team or tag" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <button className="btn" disabled={!upcoming.data?.length} onClick={() => downloadShifts(upcoming.data ?? [])} title="Your shifts of the next month as an .ics calendar file">
          <Upload size={14} /> Export My Shifts
        </button>
        <button className="btn btn-primary" onClick={() => navigate('/on-call/schedules/new')}>
          <Plus size={15} /> New Schedule
        </button>
      </div>
      <div className={`onc-split${facetOpen ? '' : ' no-controls'}`}>
        {facetOpen && <aside className="onc-side">{facet}</aside>}
        <div className="onc-main">
          <div className="onc-main-bar">
            {!facetOpen && facet}
            <Segmented
              value={scope}
              onChange={setScope}
              options={[
                ['mine', 'My Schedules'],
                ['all', 'All Schedules'],
              ]}
            />
            <span className="onc-bar-sep" />
            <Showing count={rows.length} noun="Schedules" />
          </div>
          {q.isPending ? (
            <Loading label="Loading schedules…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : !rows.length ? (
            <div className="onc-empty">
              <CalendarDays className="onc-empty-icon" size={64} strokeWidth={1.2} />
              <h3>No schedules found</h3>
              <p className="muted">{filtered ? 'Check your search query or clear filters' : 'Create a schedule to rotate who is on call.'}</p>
              {filtered ? (
                <button className="btn btn-primary btn-sm" onClick={clear}>
                  Clear Filters
                </button>
              ) : (
                <button className="btn btn-primary" onClick={() => navigate('/on-call/schedules/new')}>
                  <Plus size={15} /> New Schedule
                </button>
              )}
            </div>
          ) : (
            <div className="onc-table-wrap">
              <table className="onc-table">
                <thead>
                  <tr>
                    <th>SCHEDULE</th>
                    <th className="onc-col-team">TEAM</th>
                    <th className="onc-col-resp">ON CALL NOW</th>
                    <th className="onc-col-num">LAYERS</th>
                    <th className="onc-col-tz">TIME ZONE</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((s) => (
                    <tr key={s.id} onClick={() => navigate(`/on-call/schedules/${s.id}`)}>
                      <td>
                        <span className="onc-link">{s.name}</span>
                        {s.tags.length > 0 && <span className="onc-tags">{s.tags.slice(0, 2).join(' · ')}</span>}
                      </td>
                      <td className="onc-col-team">{s.team ? `@${s.team}` : <span className="muted">—</span>}</td>
                      <td className="onc-col-resp">
                        <People users={s.onCallNow} empty="Nobody" />
                      </td>
                      <td className="onc-col-num">{s.layers.length}</td>
                      <td className="onc-col-tz muted">{s.timeZone}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

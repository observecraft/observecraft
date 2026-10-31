import { useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Search } from 'lucide-react';
import type { SessionFilter } from '../../../api/rum/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { Select } from '../../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { useSelectedApplication } from '../common';
import { useReplayHistory, useRumEvents } from '../queries';
import { RecordingsTable } from './common';

const minute = 60_000;

const TIME_SPENT: [string, string][] = [
  ['', 'Any'],
  ['10000', '10s'],
  ['30000', '30s'],
  ['60000', '1m'],
  ['300000', '5m'],
  ['600000', '10m'],
];
const ACTIONS: [string, string][] = [
  ['', 'Any'],
  ['1', '1'],
  ['5', '5'],
  ['10', '10'],
  ['25', '25'],
];

/** Search Recordings: the sessions with a recording, filtered by who, how long and what users did. */
export function RecordingsPage() {
  const [params, setParams] = useSearchParams();
  const [selected] = useSelectedApplication();
  const { range } = useTime();
  const set = (k: string, v: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const user = params.get('user') ?? '';
  const did = params.get('q') ?? '';
  const active = params.get('active') ?? '';
  const minTime = params.get('time') ?? '';
  const minActions = params.get('actions') ?? '';
  const [userDraft, setUserDraft] = useState(user);
  const [didDraft, setDidDraft] = useState(did);

  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const sessions: SessionFilter = {
    withReplay: true,
    active: active ? active === 'true' : null,
    minTimeSpentMs: minTime ? Number(minTime) : null,
    minActionCount: minActions ? Number(minActions) : null,
  };
  // A user is found by name, or by email when the search has an @.
  const who = user.trim().replace(/[\\"]/g, '');
  const query = [who && `@usr.${who.includes('@') ? 'email' : 'name'}:"${who}"`, did.trim()].filter(Boolean).join(' ');
  const list = useRumEvents({ range: r, query, eventType: 'session', applicationId: selected, limit: 100, sessions });
  const history = useReplayHistory();
  const watched = useMemo(() => new Set((history.data ?? []).map((w) => w.sessionId)), [history.data]);

  return (
    <div className="replay-page">
      <div className="replay-toolbar">
        <form
          className="replay-search"
          onSubmit={(e) => {
            e.preventDefault();
            set('user', userDraft);
          }}
        >
          <Search size={14} />
          <input placeholder="Search for a user by name or email" value={userDraft} onChange={(e) => setUserDraft(e.target.value)} onBlur={() => set('user', userDraft)} />
        </form>
        <TimePicker compact />
      </div>
      <div className="replay-filters">
        <Select
          label="Is Active"
          value={active}
          onChange={(v) => set('active', v)}
          options={[
            { value: '', label: 'Any' },
            { value: 'true', label: 'Active' },
            { value: 'false', label: 'Ended' },
          ]}
        />
        <Select label="Time Spent >=" value={minTime} onChange={(v) => set('time', v)} options={TIME_SPENT.map(([value, label]) => ({ value, label }))} />
        <Select label="Action Count >=" value={minActions} onChange={(v) => set('actions', v)} options={ACTIONS.map(([value, label]) => ({ value, label }))} />
        <form
          className="replay-did"
          onSubmit={(e) => {
            e.preventDefault();
            set('q', didDraft);
          }}
        >
          <span>Where users did</span>
          <input placeholder='@view.name:/checkout @action.target.name:"Place order"' value={didDraft} onChange={(e) => setDidDraft(e.target.value)} onBlur={() => set('q', didDraft)} />
        </form>
      </div>
      {list.isLoading ? (
        <Loading />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : !list.data?.rows.length ? (
        <Empty>
          <b>No recordings match.</b>
          <p className="muted">Recordings come from browser sessions sampled for Session Replay. Widen the time range or remove filters.</p>
        </Empty>
      ) : (
        <>
          <p className="muted replay-count">
            {compact(list.data.total)} recording{list.data.total === 1 ? '' : 's'}
          </p>
          <RecordingsTable rows={list.data.rows} watched={watched} />
        </>
      )}
    </div>
  );
}

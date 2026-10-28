import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { CalendarDays, ChevronLeft, ChevronRight, Download, PhoneCall, PhoneOff, Users, X } from 'lucide-react';
import { DAY, monthGrid, shiftsToIcs, utcToWall, wallToUtc } from '../../api/onCall/logic';
import type { OnCallMyShift } from '../../api/onCall/client';
import { Select } from '../../components/ui/Dropdown';
import { EmptyState, PagesTable, Segmented, Toggle, ViewAll, browserTimeZone, timeZoneOptions } from './common';
import { ProfileSetup } from './ProfileSetup';
import { useMyOnCallShifts, useOnCallPages, useOnCallPolicies, useOnCallProfile, useOnCallTeams } from './queries';

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const DOW = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function useDismissed(key: string): [boolean, () => void] {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(key) === '1';
    } catch {
      return false;
    }
  });
  return [
    dismissed,
    () => {
      setDismissed(true);
      try {
        localStorage.setItem(key, '1');
      } catch {
        /* storage unavailable */
      }
    },
  ];
}

/** On-Call → Summary: getting set up, the org's latest pages, my shifts and a nudge to route a team. */
export function SummaryPage() {
  const navigate = useNavigate();
  const profile = useOnCallProfile();
  const teams = useOnCallTeams();
  const [setupHidden, hideSetup] = useDismissed('oc.onCall.setupHidden');
  const [calloutHidden, hideCallout] = useDismissed('oc.onCall.teamCalloutHidden');
  const myTeams = (teams.data ?? []).filter((t) => t.mine);
  const routed = myTeams.some((t) => t.policy);
  return (
    <div className="onc-summary">
      <div className="onc-hero">
        <div className="onc-kicker">
          <PhoneCall size={14} /> On-Call
        </div>
        <h1>Stay ahead of outages: the right person hears about it first, and someone always answers.</h1>
      </div>
      {!setupHidden && profile.data && !profile.data.configured && (
        <section className="onc-card onc-setup">
          <div className="onc-setup-band">
            <span>Get notified when a page is triggered</span>
            <button className="link-btn" onClick={hideSetup}>
              Set up your profile later
            </button>
          </div>
          <ProfileSetup />
        </section>
      )}
      <LatestPages teams={(teams.data ?? []).map((t) => ({ handle: t.handle, name: t.name }))} />
      <MyShifts />
      {!calloutHidden && teams.data && !routed && (
        <section className="onc-callout">
          <Users size={28} strokeWidth={1.5} />
          <div>
            <h3>Set up a team to get paged</h3>
            <p className="muted">Give a team an escalation policy, so pages for it reach whoever is on call and escalate when nobody answers.</p>
          </div>
          <button className="btn btn-primary" onClick={() => navigate('/on-call/teams')}>
            Set Up a Team
          </button>
          <button className="icon-plain onc-callout-close" title="Dismiss" onClick={hideCallout}>
            <X size={16} />
          </button>
        </section>
      )}
    </div>
  );
}

function LatestPages({ teams }: { teams: { handle: string; name: string }[] }) {
  const navigate = useNavigate();
  const [team, setTeam] = useState('*');
  const [mine, setMine] = useState(false);
  const q = useOnCallPages({ status: 'all', teams: team === '*' ? [] : [team], mine, query: '', limit: 5 });
  return (
    <section className="onc-card">
      <div className="onc-card-head">
        <h2>Latest On-Call Pages in the organization</h2>
        <div className="onc-card-tools">
          <Select label="Teams" value={team} onChange={setTeam} options={[{ value: '*', label: 'All teams' }, ...teams.map((t) => ({ value: t.handle, label: t.name }))]} />
          <Toggle on={mine} onChange={setMine} label="Only show pages assigned to me" />
          <ViewAll to="/on-call/pages" />
        </div>
      </div>
      <PagesTable
        compact
        pages={q.data?.pages ?? []}
        onOpen={(p) => navigate(`/on-call/pages/${p.id}`)}
        empty={<EmptyState icon={PhoneOff} title={q.isPending ? 'Loading pages…' : 'No Pages found'} />}
      />
    </section>
  );
}

/** My shifts per local day of a month (Sunday-first grid). */
function shiftsByDay(shifts: OnCallMyShift[], tz: string, days: number[]): Map<number, OnCallMyShift[]> {
  const out = new Map<number, OnCallMyShift[]>();
  for (const day of days) {
    const from = wallToUtc(tz, day);
    const to = wallToUtc(tz, day + DAY);
    out.set(
      day,
      shifts.filter((s) => s.end > from && s.start < to),
    );
  }
  return out;
}

function MyShifts() {
  const tzDefault = browserTimeZone();
  const [tz, setTz] = useState(tzDefault);
  const [mode, setMode] = useState<'pageable' | 'all'>('pageable');
  const nowWall = utcToWall(tz, Date.now());
  const [month, setMonth] = useState(() => ({ y: new Date(nowWall).getUTCFullYear(), m: new Date(nowWall).getUTCMonth() }));
  const weeks = useMemo(() => monthGrid(month.y, month.m), [month]);
  const range = useMemo(() => ({ from: wallToUtc(tz, weeks[0][0].wall), to: wallToUtc(tz, weeks[weeks.length - 1][6].wall + DAY) }), [weeks, tz]);
  const q = useMyOnCallShifts(range);
  // The next month from now, for the empty text and the export.
  const minute = Math.floor(Date.now() / 60_000) * 60_000;
  const upcomingRange = useMemo(() => ({ from: minute, to: minute + 31 * DAY }), [minute]);
  const upcoming = useMyOnCallShifts(upcomingRange);
  const policies = useOnCallPolicies();
  const pageable = useMemo(() => new Set((policies.data ?? []).flatMap((p) => p.steps.flatMap((s) => s.targets.filter((t) => t.kind === 'schedule').map((t) => t.id)))), [policies.data]);
  const keep = (s: OnCallMyShift) => mode === 'all' || pageable.has(s.scheduleId);
  const shifts = (q.data ?? []).filter(keep);
  const next = (upcoming.data ?? []).filter(keep);
  const byDay = shiftsByDay(
    shifts,
    tz,
    weeks.flat().map((d) => d.wall),
  );
  const today = Date.UTC(new Date(nowWall).getUTCFullYear(), new Date(nowWall).getUTCMonth(), new Date(nowWall).getUTCDate());
  const step = (d: -1 | 1) => setMonth(({ y, m }) => ({ y: m + d < 0 ? y - 1 : m + d > 11 ? y + 1 : y, m: (m + d + 12) % 12 }));
  const exportIcs = () => {
    const blob = new Blob([shiftsToIcs(next, Date.now(), location.hostname || 'observecraft')], { type: 'text/calendar' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'my-on-call-shifts.ics';
    a.click();
    URL.revokeObjectURL(a.href);
  };
  return (
    <section className="onc-card">
      <div className="onc-card-head">
        <h2>My Shifts</h2>
        <div className="onc-card-tools">
          <Segmented
            value={mode}
            onChange={setMode}
            options={[
              ['pageable', 'Only show pageable shifts'],
              ['all', 'Show all shifts'],
            ]}
          />
          <Select label="Time Zone" value={tz} onChange={setTz} options={timeZoneOptions(tz).map((z) => ({ value: z, label: z }))} />
          <button className="btn" disabled={!next.length} onClick={exportIcs} title="Your shifts of the next month as an .ics calendar file">
            <Download size={14} /> Export My Shifts
          </button>
        </div>
      </div>
      <div className="onc-cal-bar">
        <button className="btn btn-sm" onClick={() => setMonth({ y: new Date(nowWall).getUTCFullYear(), m: new Date(nowWall).getUTCMonth() })}>
          Today
        </button>
        <button className="icon-btn" title="Previous month" onClick={() => step(-1)}>
          <ChevronLeft size={16} />
        </button>
        <button className="icon-btn" title="Next month" onClick={() => step(1)}>
          <ChevronRight size={16} />
        </button>
        <b>
          {MONTHS[month.m]} {month.y}
        </b>
        {upcoming.data && !next.length && <span className="muted onc-cal-empty">You don&apos;t have any upcoming shifts within the next 1 month</span>}
      </div>
      <div className="onc-cal">
        {DOW.map((d) => (
          <div key={d} className="onc-cal-dow">
            {d}
          </div>
        ))}
        {weeks.flat().map((d) => (
          <div key={d.wall} className={`onc-cal-day${d.inMonth ? '' : ' out'}${d.wall === today ? ' today' : ''}`}>
            <span className="onc-cal-num">{d.day}</span>
            {(byDay.get(d.wall) ?? []).slice(0, 3).map((s) => (
              <span key={`${s.scheduleId}-${s.start}`} className="onc-cal-shift" title={`${s.scheduleName}: ${new Date(s.start).toLocaleString()} → ${new Date(s.end).toLocaleString()}`}>
                <CalendarDays size={11} /> {s.scheduleName}
              </span>
            ))}
          </div>
        ))}
      </div>
    </section>
  );
}

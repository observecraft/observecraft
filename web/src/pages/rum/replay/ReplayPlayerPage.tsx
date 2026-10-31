import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link, useParams, useSearchParams } from 'react-router-dom';
import { AlertCircle, ArrowLeft, ChevronDown, Eye, FastForward, Link2, ListPlus, MousePointerClick, Pause, Play, RotateCcw, Square } from 'lucide-react';
import type { RumEventRow } from '../../../api/rum/client';
import { Menu } from '../../../components/ui/Dropdown';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { fmtMsShort, fmtNs } from '../common';
import { useMarkWatched, usePlaylistSession, useReplayPlaylists, useSavePlaylist, useSessionReplay } from '../queries';
import { attr, clock, ReplayStage, userLabel } from './common';
import { flatten, RecordType, Replayer, type Cursor } from './replayer';

const SPEEDS = [1, 2, 4, 8];
/** Pauses longer than this between interactions are skipped when "Skip inactivity" is on. */
const INACTIVE_MS = 3000;

/** The player of one recorded session: the page as the user saw it, its timeline and its events. */
export function ReplayPlayerPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const startedAt = Number(params.get('ts')) || Date.now();
  const replay = useSessionReplay(id, startedAt);
  const watch = useMarkWatched();
  const marked = useRef(false);
  useEffect(() => {
    if (replay.data && id && !marked.current) {
      marked.current = true;
      watch.mutate({ id, sessionId: id, startedAt });
    }
  }, [replay.data, id, startedAt, watch]);

  if (replay.isLoading) return <Loading label="Loading the recording…" />;
  if (replay.isError) return <ErrorState error={replay.error} onRetry={() => replay.refetch()} />;
  const data = replay.data!;
  return <Player key={id} id={id!} startedAt={startedAt} session={data.session ?? null} segments={data.segments} events={data.events} watched={data.watched} />;
}

interface Props {
  id: string;
  startedAt: number;
  session: RumEventRow | null;
  segments: { records: unknown[] }[];
  events: RumEventRow[];
  watched: number;
}

function Player({ id, startedAt, session, segments, events, watched }: Props) {
  const records = useMemo(() => flatten(segments), [segments]);
  const frame = useRef<HTMLIFrameElement>(null);
  const replayer = useRef<Replayer | null>(null);
  const [viewport, setViewport] = useState({ w: 1280, h: 800 });
  const [cursor, setCursor] = useState<Cursor | null>(null);
  const [clicks, setClicks] = useState<(Cursor & { key: number })[]>([]);
  const start = records[0]?.timestamp ?? startedAt;
  const end = Math.max(records.at(-1)?.timestamp ?? start, start + 1);
  const [time, setTime] = useState(start);
  const timeRef = useRef(start);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [skip, setSkip] = useState(true);
  const [tab, setTab] = useState<'events' | 'details'>('events');

  // Moments of user activity, to skip the pauses between them.
  const activity = useMemo(() => records.filter((r) => r.type !== RecordType.Focus).map((r) => r.timestamp), [records]);

  useEffect(() => {
    if (!frame.current || !records.length) return;
    let n = 0;
    const r = new Replayer(frame.current, records, {
      onViewport: (w, h) => setViewport({ w, h }),
      onCursor: setCursor,
      onClick: (c) => {
        const key = ++n;
        setClicks((cs) => [...cs.slice(-8), { ...c, key }]);
        setTimeout(() => setClicks((cs) => cs.filter((x) => x.key !== key)), 700);
      },
    });
    replayer.current = r;
    r.seek(start);
    timeRef.current = start;
    setTime(start);
    setPlaying(true);
  }, [records, start]);

  const seek = useCallback(
    (t: number) => {
      const c = Math.min(Math.max(t, start), end);
      setClicks([]);
      replayer.current?.seek(c);
      timeRef.current = c;
      setTime(c);
    },
    [start, end],
  );

  useEffect(() => {
    if (!playing) return;
    let last = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      let t = timeRef.current + (now - last) * speed;
      last = now;
      if (skip) {
        const next = activity.find((a) => a > timeRef.current);
        if (next !== undefined && next - timeRef.current > INACTIVE_MS) t = Math.max(t, next - 300);
      }
      if (t >= end) {
        replayer.current?.advance(end);
        timeRef.current = end;
        setTime(end);
        setPlaying(false);
        return;
      }
      replayer.current?.advance(t);
      timeRef.current = t;
      setTime(t);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, speed, skip, end, activity]);

  // Space plays and pauses, arrows move 5 seconds.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest('input, textarea, select')) return;
      if (e.key === ' ') {
        e.preventDefault();
        setPlaying((p) => !p);
      } else if (e.key === 'ArrowRight') seek(timeRef.current + 5000);
      else if (e.key === 'ArrowLeft') seek(timeRef.current - 5000);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [seek]);

  const pct = (t: number) => `${(Math.min(Math.max(t - start, 0), end - start) / (end - start)) * 100}%`;
  const current = useMemo(() => {
    let i = -1;
    events.forEach((e, n) => e.timestamp <= time && (i = n));
    return i;
  }, [events, time]);
  const view = [...events].reverse().find((e) => e.type === 'view' && e.timestamp <= time) ?? events.find((e) => e.type === 'view');

  return (
    <div className="replay-player-page">
      <header className="replay-player-head">
        <Link to="/rum/replay/sessions" className="btn btn-ghost" title="Back to the recordings">
          <ArrowLeft size={16} />
        </Link>
        <div className="replay-player-title">
          <b>{session ? userLabel(session) : 'Session'}</b>
          <span className="muted">
            {new Date(startedAt).toLocaleString()} · {attr(session, 'browser.name')} {attr(session, 'os.name') && `· ${attr(session, 'os.name')}`}{' '}
            {attr(session, 'geo.country') && `· ${attr(session, 'geo.country')}`}
          </span>
        </div>
        <span className="muted replay-watched" title="People who watched this recording">
          <Eye size={14} /> {watched}
        </span>
        <AddToPlaylist id={id} startedAt={startedAt} />
        <button className="btn" onClick={() => navigator.clipboard?.writeText(location.href)} title="Copy the link to this recording">
          <Link2 size={14} /> Share
        </button>
        <Link className="btn" to={`/rum/explorer?type=all&q=${encodeURIComponent(`@session.id:${id}`)}`}>
          View in RUM
        </Link>
      </header>
      <div className="replay-player-body">
        <section className="replay-main">
          <div className="replay-url mono">{view ? attr(view, 'view.url') || attr(view, 'view.name') : ''}</div>
          {records.length ? (
            <ReplayStage viewport={viewport} frameRef={frame}>
              {cursor && <div className="replay-cursor" style={{ left: cursor.x, top: cursor.y }} />}
              {clicks.map((c) => (
                <div key={c.key} className="replay-click" style={{ left: c.x, top: c.y }} />
              ))}
            </ReplayStage>
          ) : (
            <div className="replay-none muted">This session has no recorded segments.</div>
          )}
          <div className="replay-controls">
            <button className="btn btn-ghost" onClick={() => (time >= end ? (seek(start), setPlaying(true)) : setPlaying((p) => !p))} title={playing ? 'Pause (space)' : 'Play (space)'}>
              {playing ? <Pause size={16} /> : time >= end ? <RotateCcw size={16} /> : <Play size={16} />}
            </button>
            <span className="mono replay-time">
              {clock(time - start)} / {clock(end - start)}
            </span>
            <div
              className="replay-timeline"
              onMouseDown={(e) => {
                const box = e.currentTarget.getBoundingClientRect();
                seek(start + ((e.clientX - box.left) / box.width) * (end - start));
              }}
            >
              <div className="replay-progress" style={{ width: pct(time) }} />
              {events
                .filter((e) => e.type !== 'view' || e.timestamp > start)
                .map((e) => (
                  <i key={e.id} className={`replay-dot ${dotKind(e)}`} style={{ left: pct(e.timestamp) }} title={eventTitle(e)} />
                ))}
              <span className="replay-head" style={{ left: pct(time) }} />
            </div>
            <Menu
              triggerClassName="btn btn-ghost replay-speed"
              trigger={
                <>
                  {speed}x <ChevronDown size={13} />
                </>
              }
              items={SPEEDS.map((s) => ({ label: `${s}x`, onClick: () => setSpeed(s) }))}
            />
            <label className="replay-skip" title="Skip the pauses between interactions">
              <input type="checkbox" checked={skip} onChange={(e) => setSkip(e.target.checked)} /> <FastForward size={14} /> Skip inactivity
            </label>
          </div>
        </section>
        <aside className="replay-side">
          <nav className="replay-side-tabs">
            <button className={tab === 'events' ? 'active' : ''} onClick={() => setTab('events')}>
              Events <span className="muted">{events.length}</span>
            </button>
            <button className={tab === 'details' ? 'active' : ''} onClick={() => setTab('details')}>
              Details
            </button>
          </nav>
          {tab === 'events' ? (
            <ol className="replay-events">
              {events.map((e, n) => (
                <li key={e.id} className={`${e.type}${n === current ? ' current' : ''}${n > current ? ' future' : ''}`}>
                  <button onClick={() => seek(e.timestamp)}>
                    <span className="replay-ev-icon">{icon(e)}</span>
                    <span className="replay-ev-text">
                      <b>{eventTitle(e)}</b>
                      {e.type === 'view' && <span className="muted">Loading time {fmtNs(attr(e, 'view.loading_time'))}</span>}
                      {e.type === 'error' && <span className="muted">{attr(e, 'error.source')}</span>}
                    </span>
                    <span className="mono muted">{clock(e.timestamp - start)}</span>
                  </button>
                </li>
              ))}
            </ol>
          ) : (
            <dl className="replay-details">
              {(
                [
                  ['User', session ? userLabel(session) : '—'],
                  ['Email', attr(session, 'usr.email') || '—'],
                  ['Session id', id],
                  ['Time spent', fmtMsShort(Number(attr(session, 'session.time_spent')))],
                  ['Views', attr(session, 'session.view.count')],
                  ['Actions', attr(session, 'session.action.count')],
                  ['Errors', attr(session, 'session.error.count')],
                  ['Frustrations', attr(session, 'session.frustration.count')],
                  ['Browser', `${attr(session, 'browser.name')} ${attr(session, 'browser.version')}`],
                  ['OS', attr(session, 'os.name')],
                  ['Device', attr(session, 'device.type')],
                  ['Location', [attr(session, 'geo.city'), attr(session, 'geo.country')].filter(Boolean).join(', ')],
                  ['Viewport', `${viewport.w} × ${viewport.h}`],
                ] as [string, string][]
              ).map(([k, v]) => (
                <div key={k}>
                  <dt>{k}</dt>
                  <dd className={k === 'Session id' ? 'mono' : ''}>{v || '—'}</dd>
                </div>
              ))}
            </dl>
          )}
        </aside>
      </div>
    </div>
  );
}

function dotKind(e: RumEventRow): string {
  if (e.type === 'action' && attr(e, 'action.frustration.type')) return 'frustration';
  return e.type;
}

function eventTitle(e: RumEventRow): string {
  switch (e.type) {
    case 'view':
      return attr(e, 'view.name') || attr(e, 'view.url');
    case 'action': {
      const f = attr(e, 'action.frustration.type');
      return `${f ? f.replace('_', ' ') + ' on' : attr(e, 'action.type') || 'click on'} "${attr(e, 'action.target.name')}"`;
    }
    case 'error':
      return attr(e, 'error.message') || 'Error';
    default:
      return e.type;
  }
}

function icon(e: RumEventRow) {
  if (e.type === 'error') return <AlertCircle size={14} />;
  if (e.type === 'action') return <MousePointerClick size={14} />;
  return <Square size={12} />;
}

/** "Add to playlist ▾": the viewer's playlists, or a new one. */
function AddToPlaylist({ id, startedAt }: { id: string; startedAt: number }) {
  const playlists = useReplayPlaylists();
  const add = usePlaylistSession();
  const create = useSavePlaylist();
  const [done, setDone] = useState<string | null>(null);
  const addTo = (playlist: string, name: string) => add.mutate({ playlist, session: { sessionId: id, startedAt } }, { onSuccess: () => setDone(name) });
  return (
    <>
      {done && <span className="muted replay-added">Added to {done}</span>}
      <Menu
        triggerClassName="btn"
        trigger={
          <>
            <ListPlus size={14} /> Add to playlist <ChevronDown size={13} />
          </>
        }
        items={[
          ...(playlists.data ?? []).map((p) => ({ label: p.name, onClick: () => addTo(p.id, p.name) })),
          {
            label: '+ New playlist…',
            onClick: () => {
              const name = prompt('Name of the new playlist');
              if (name?.trim()) create.mutate({ name: name.trim() }, { onSuccess: (p) => addTo(p.id, p.name) });
            },
          },
        ]}
      />
    </>
  );
}

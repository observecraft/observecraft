import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { Link } from 'react-router-dom';
import { Eye, Play } from 'lucide-react';
import type { RumEventRow } from '../../../api/rum/client';
import { fmtMsShort } from '../common';

export const attr = (r: RumEventRow | null | undefined, k: string) => r?.attributes[k] ?? '';

/** Who the session belongs to: name, email, or an anonymous visitor. */
export function userLabel(r: RumEventRow): string {
  const anon = attr(r, 'usr.anonymous_id');
  return attr(r, 'usr.name') || attr(r, 'usr.email') || (anon ? `Anonymous user ${anon.slice(0, 6)}` : 'Anonymous user');
}

/** The player's address of a recorded session. */
export const playerPath = (id: string, startedAt: number) => `/rum/replay/sessions/${id}?ts=${startedAt}`;

/** 75 seconds as 1:15. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor((s % 3600) / 60));
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${mm.padStart(2, '0')}:${ss}` : `${mm}:${ss}`;
}

const date = (t: number) => new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false });

/** The recorded page, scaled to fit its box: a sandboxed iframe (no scripts) with an overlay for the cursor and clicks. */
export function ReplayStage({
  viewport,
  frameRef,
  children,
  fit = 'contain',
}: {
  viewport: { w: number; h: number };
  frameRef: RefObject<HTMLIFrameElement>;
  children?: ReactNode;
  fit?: 'contain' | 'width';
}) {
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const ro = new ResizeObserver(([e]) => setSize({ w: e.contentRect.width, h: e.contentRect.height }));
    ro.observe(box.current!);
    return () => ro.disconnect();
  }, []);
  const scale = size.w ? (fit === 'width' ? size.w / viewport.w : Math.min(size.w / viewport.w, size.h / viewport.h)) : 0;
  return (
    <div className={`replay-stage ${fit}`} ref={box} style={fit === 'width' ? { height: viewport.h * scale } : undefined}>
      <div className="replay-canvas" style={{ width: viewport.w, height: viewport.h, transform: `scale(${scale})`, left: Math.max(0, (size.w - viewport.w * scale) / 2) }}>
        <iframe ref={frameRef} sandbox="allow-same-origin" title="Recorded page" width={viewport.w} height={viewport.h} tabIndex={-1} />
        <div className="replay-overlay">{children}</div>
      </div>
    </div>
  );
}

/** Recorded sessions, newest first: Watch, user, platform, pages and counts. */
export function RecordingsTable({ rows, watched, actions }: { rows: RumEventRow[]; watched?: Set<string>; actions?: (r: RumEventRow) => ReactNode }) {
  return (
    <div className="mon-table-wrap">
      <table className="mon-table syn-table replay-table">
        <thead>
          <tr>
            <th />
            <th>DATE</th>
            <th>USER</th>
            <th className="num">TIME SPENT</th>
            <th>BROWSER</th>
            <th>OS</th>
            <th>COUNTRY</th>
            <th>FIRST PAGE</th>
            <th>LAST PAGE</th>
            <th className="num">VIEWS</th>
            <th className="num">ACTIONS</th>
            <th className="num">ERRORS</th>
            <th className="num">FRUSTRATIONS</th>
            {actions && <th />}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const id = attr(r, 'session.id') || r.id;
            const seen = watched?.has(id);
            return (
              <tr key={id}>
                <td>
                  <Link className={`btn replay-watch${seen ? ' seen' : ''}`} to={playerPath(id, r.timestamp)} title={seen ? 'Watched' : 'Watch the recording'}>
                    {seen ? <Eye size={14} /> : <Play size={14} />} Watch
                  </Link>
                </td>
                <td className="nowrap">{date(r.timestamp)}</td>
                <td className="replay-user">{userLabel(r)}</td>
                <td className="num">{fmtMsShort(Number(attr(r, 'session.time_spent')))}</td>
                <td>{attr(r, 'browser.name')}</td>
                <td>{attr(r, 'os.name')}</td>
                <td>{attr(r, 'geo.country')}</td>
                <td className="mono">{attr(r, 'session.initial_view.name')}</td>
                <td className="mono">{attr(r, 'session.last_view.name')}</td>
                <td className="num">{attr(r, 'session.view.count')}</td>
                <td className="num">{attr(r, 'session.action.count')}</td>
                <td className={`num${Number(attr(r, 'session.error.count')) ? ' replay-bad' : ''}`}>{attr(r, 'session.error.count')}</td>
                <td className="num">{attr(r, 'session.frustration.count')}</td>
                {actions && <td className="nowrap">{actions(r)}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

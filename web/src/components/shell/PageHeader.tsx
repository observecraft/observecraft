import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronDown, FastForward, Pause, Pin, Play, RefreshCw, Rewind, ZoomOut } from 'lucide-react';
import { RELATIVE_RANGES, useTime } from './TimeContext';

export function utcOffset(): string {
  const mins = -new Date().getTimezoneOffset();
  const sign = mins >= 0 ? '+' : '-';
  const abs = Math.abs(mins);
  return `UTC${sign}${String(Math.floor(abs / 60)).padStart(2, '0')}:${String(abs % 60).padStart(2, '0')}`;
}

/** `compact` drops the playback buttons (back / pause / forward / zoom), keeping picker + refresh. */
export function TimePicker({ compact = false }: { compact?: boolean }) {
  const t = useTime();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);

  return (
    <div className="time-controls">
      <div className="time-picker" ref={ref}>
        <span className="tz">{utcOffset()}</span>
        <button className="time-picker-btn" onClick={() => setOpen((o) => !o)}>
          <span className="time-short">{t.relative.short}</span>
          <span className="time-label">{t.relative.label}</span>
          <Pin size={13} className="muted" />
          <ChevronDown size={14} className="muted" />
        </button>
        {open && (
          <ul className="time-menu">
            {RELATIVE_RANGES.map((r) => (
              <li key={r.key}>
                <button
                  className={r.key === t.relative.key ? 'selected' : ''}
                  onClick={() => {
                    t.setRelative(r.key);
                    setOpen(false);
                  }}
                >
                  <span className="time-short">{r.short}</span>
                  {r.label}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {!compact && (
        <div className="btn-group">
          <button className="btn time-icon" title="Back" onClick={() => t.shift(-1)}>
            <Rewind size={14} fill="currentColor" />
          </button>
          <button
            className={`btn time-icon${t.live ? ' btn-active' : ''}`}
            title={t.live ? 'Pause live updates' : 'Resume live updates'}
            onClick={() => t.setLive(!t.live)}
          >
            {t.live ? <Pause size={14} fill="currentColor" /> : <Play size={14} fill="currentColor" />}
          </button>
          <button className="btn time-icon" title="Forward" onClick={() => t.shift(1)}>
            <FastForward size={14} fill="currentColor" />
          </button>
        </div>
      )}
      {!compact && (
        <button className="btn time-icon" title="Zoom out" onClick={t.zoomOut}>
          <ZoomOut size={14} />
        </button>
      )}
      <button className="btn time-icon" title="Refresh" onClick={t.refresh}>
        <RefreshCw size={14} />
      </button>
    </div>
  );
}

/** Title strip at the top of each page: tabs/title on the left, time picker on the right. */
export function PageHeader({ children, showTime = true }: { children: ReactNode; showTime?: boolean }) {
  return (
    <div className="page-header">
      <div className="page-header-left">{children}</div>
      {showTime && <TimePicker />}
    </div>
  );
}

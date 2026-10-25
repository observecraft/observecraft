import { useState } from 'react';
import { X } from 'lucide-react';
import type { DowntimeInput } from '../../api/monitors/client';
import { Modal } from '../../components/ui/Modal';
import { useCreateDowntime, useMonitors } from './queries';

/** `<input type="datetime-local">` value for an epoch ms, in local time. */
function localInput(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

const DURATIONS = [
  { label: '1 hour', ms: 3600e3 },
  { label: '4 hours', ms: 4 * 3600e3 },
  { label: '1 day', ms: 24 * 3600e3 },
  { label: '1 week', ms: 7 * 24 * 3600e3 },
];

export function ScheduleDowntimeModal({ onClose, monitorId }: { onClose: () => void; monitorId?: number }) {
  const create = useCreateDowntime();
  const monitors = useMonitors({
    query: '',
    statuses: [],
    muted: [],
    priorities: [],
    types: [],
    drafts: [],
    recipients: [],
    creators: [],
    teams: [],
    sort: 'name',
    offset: 0,
    limit: 200,
  });
  const [target, setTarget] = useState<'monitor' | 'tags'>(monitorId ? 'monitor' : 'monitor');
  const [monitor, setMonitor] = useState(monitorId ? String(monitorId) : '');
  const [tags, setTags] = useState('');
  const [scope, setScope] = useState('*');
  const [kind, setKind] = useState<'once' | 'recurring'>('once');
  const [start, setStart] = useState(() => localInput(Date.now()));
  const [endMode, setEndMode] = useState<'duration' | 'at' | 'never'>('duration');
  const [duration, setDuration] = useState(DURATIONS[0].ms);
  const [end, setEnd] = useState(() => localInput(Date.now() + 3600e3));
  const [repeat, setRepeat] = useState<'daily' | 'weekly'>('daily');
  const [hours, setHours] = useState('2');
  const [until, setUntil] = useState('');
  const [message, setMessage] = useState('');
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    setError(null);
    const s = new Date(start).getTime();
    if (Number.isNaN(s)) return setError('Pick a start time.');
    let e: number | null = null;
    if (kind === 'recurring') {
      const h = Number(hours);
      if (!(h > 0)) return setError('Each occurrence needs a duration.');
      e = s + h * 3600e3;
    } else if (endMode === 'duration') {
      e = s + duration;
    } else if (endMode === 'at') {
      e = new Date(end).getTime();
    }
    if (target === 'monitor' && !monitor) return setError('Pick a monitor.');
    const input: DowntimeInput = {
      monitorId: target === 'monitor' ? Number(monitor) : null,
      monitorTags:
        target === 'tags'
          ? tags
              .split(',')
              .map((t) => t.trim())
              .filter(Boolean)
          : [],
      scope: scope
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean),
      message,
      start: s,
      end: e,
      recurrence: kind === 'recurring' ? repeat : null,
      recurUntil: kind === 'recurring' && until ? new Date(until).getTime() : null,
    };
    create.mutate(input, {
      onSuccess: onClose,
      onError: (err) => setError((err as Error).message),
    });
  };

  return (
    <Modal onClose={onClose} label="Schedule downtime" width={640} height="auto">
      <form
        className="small-modal mon-dt-form"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="modal-title-row">
          <h3>Schedule Downtime</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <fieldset>
          <legend>Choose what to silence</legend>
          <div className="mon-seg">
            <button type="button" className={target === 'monitor' ? 'active' : ''} onClick={() => setTarget('monitor')}>
              By Monitor Name
            </button>
            <button type="button" className={target === 'tags' ? 'active' : ''} onClick={() => setTarget('tags')}>
              By Monitor Tags
            </button>
          </div>
          {target === 'monitor' ? (
            <select value={monitor} onChange={(e) => setMonitor(e.target.value)} aria-label="Monitor">
              <option value="">Select a monitor…</option>
              {(monitors.data?.monitors ?? []).map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
          ) : (
            <input placeholder="team:platform, env:prod (empty = every monitor)" value={tags} onChange={(e) => setTags(e.target.value)} aria-label="Monitor tags" />
          )}
          <label className="form-label">
            Group scope
            <input placeholder="* or host:web-1, service:billing" value={scope} onChange={(e) => setScope(e.target.value)} />
          </label>
        </fieldset>
        <fieldset>
          <legend>Schedule</legend>
          <div className="mon-seg">
            <button type="button" className={kind === 'once' ? 'active' : ''} onClick={() => setKind('once')}>
              One Time
            </button>
            <button type="button" className={kind === 'recurring' ? 'active' : ''} onClick={() => setKind('recurring')}>
              Recurring
            </button>
          </div>
          <label className="form-label">
            {kind === 'recurring' ? 'First occurrence starts' : 'Start'}
            <input type="datetime-local" value={start} onChange={(e) => setStart(e.target.value)} required />
          </label>
          {kind === 'once' ? (
            <div className="mon-dt-end">
              <label>
                <input type="radio" checked={endMode === 'duration'} onChange={() => setEndMode('duration')} /> For
                <select value={duration} onChange={(e) => setDuration(Number(e.target.value))} disabled={endMode !== 'duration'}>
                  {DURATIONS.map((d) => (
                    <option key={d.ms} value={d.ms}>
                      {d.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <input type="radio" checked={endMode === 'at'} onChange={() => setEndMode('at')} /> Until
                <input type="datetime-local" value={end} onChange={(e) => setEnd(e.target.value)} disabled={endMode !== 'at'} />
              </label>
              <label>
                <input type="radio" checked={endMode === 'never'} onChange={() => setEndMode('never')} /> Until canceled
              </label>
            </div>
          ) : (
            <div className="mon-dt-end">
              <label>
                Repeat
                <select value={repeat} onChange={(e) => setRepeat(e.target.value as 'daily' | 'weekly')}>
                  <option value="daily">every day</option>
                  <option value="weekly">every week</option>
                </select>
                for
                <input className="mon-num" type="number" min={0.25} step={0.25} value={hours} onChange={(e) => setHours(e.target.value)} /> hours
              </label>
              <label>
                Stop repeating after
                <input type="datetime-local" value={until} onChange={(e) => setUntil(e.target.value)} />
                <span className="muted">(optional)</span>
              </label>
            </div>
          )}
        </fieldset>
        <label className="form-label">
          Message
          <textarea rows={2} placeholder="Why is this silenced? e.g. Planned database upgrade" value={message} onChange={(e) => setMessage(e.target.value)} />
        </label>
        <p className="muted">Monitors keep evaluating during a downtime; their notifications are suppressed.</p>
        {error && <p className="form-error">{error}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={create.isPending}>
            Schedule
          </button>
        </div>
      </form>
    </Modal>
  );
}

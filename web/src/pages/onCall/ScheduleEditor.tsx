import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { AlertTriangle, ArrowDown, ArrowUp, CalendarDays, ChevronLeft, ChevronRight, Copy, GripVertical, Layers, Pencil, PhoneCall, Plus, Trash2, X } from 'lucide-react';
import type { OnCallLayer, OnCallRestriction, OnCallScheduleInput, OnCallScheduleShifts, OnCallWeekday } from '../../api/onCall/client';
import { DAY, HOUR, WEEKDAYS, colorSlot, dayColumns, defaultScheduleName, formatLocal, layerProblem, parseLocal, rotationLabel, timelineBlock, utcToWall, wallToUtc } from '../../api/onCall/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { useProfile, useUsers } from '../settings/queries';
import { browserTimeZone, errorText, timeZoneOptions } from './common';
import { useCreateOnCallOverride, useDeleteOnCallOverride, useDeleteOnCallSchedule, useOnCallPreview, useOnCallSchedule, useOnCallTeams, useSaveOnCallSchedule } from './queries';
import '../monitors/monitors.css';
import './onCall.css';

type Frame = 'day' | 'week' | '2w' | 'month';
const FRAMES: { value: Frame; label: string }[] = [
  { value: 'day', label: 'Day' },
  { value: 'week', label: 'Week' },
  { value: '2w', label: '2 Weeks' },
  { value: 'month', label: 'Month' },
];
const DAY_LABEL: Record<OnCallWeekday, string> = { mon: 'Monday', tue: 'Tuesday', wed: 'Wednesday', thu: 'Thursday', fri: 'Friday', sat: 'Saturday', sun: 'Sunday' };

const midnightWall = (wall: number) => wall - (((wall % DAY) + DAY) % DAY);

function frameStart(frame: Frame, wall: number): number {
  const day = midnightWall(wall);
  if (frame === 'day') return day;
  if (frame === 'month') return Date.UTC(new Date(day).getUTCFullYear(), new Date(day).getUTCMonth(), 1);
  return day - new Date(day).getUTCDay() * DAY;
}

function frameEnd(frame: Frame, start: number): number {
  if (frame === 'day') return start + DAY;
  if (frame === 'week') return start + 7 * DAY;
  if (frame === '2w') return start + 14 * DAY;
  return Date.UTC(new Date(start).getUTCFullYear(), new Date(start).getUTCMonth() + 1, 1);
}

function stepFrame(frame: Frame, start: number, dir: -1 | 1): number {
  if (frame === 'month') return Date.UTC(new Date(start).getUTCFullYear(), new Date(start).getUTCMonth() + dir, 1);
  return start + dir * (frame === 'day' ? 1 : frame === 'week' ? 7 : 14) * DAY;
}

const fmtDay = (wall: number, opts: Intl.DateTimeFormatOptions) => new Date(wall).toLocaleDateString('en-US', { ...opts, timeZone: 'UTC' });

function newLayer(index: number, now: number, tz: string): OnCallLayer {
  const day = midnightWall(utcToWall(tz, now));
  return {
    id: `layer-${Date.now().toString(36)}-${index}`,
    name: `Layer ${index + 1}`,
    members: [],
    rotationHours: 168,
    effectiveFrom: formatLocal(day + 9 * HOUR),
    effectiveUntil: null,
    restrictions: [],
  };
}

/** On-Call → Schedules → New Schedule / a schedule: layers on a timeline, the final schedule and overrides. */
export function ScheduleEditor() {
  const { id } = useParams();
  const q = useOnCallSchedule(id);
  if (id && q.isPending) return <Loading label="Loading schedule…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const initial: OnCallScheduleInput | null = q.data
    ? { name: q.data.name, timeZone: q.data.timeZone, team: q.data.team, tags: q.data.tags, layers: q.data.layers, maxConcurrent: q.data.maxConcurrent }
    : null;
  return <Editor key={id ?? 'new'} id={id} initial={initial} />;
}

function Editor({ id, initial }: { id?: string; initial: OnCallScheduleInput | null }) {
  const navigate = useNavigate();
  const me = useProfile();
  const users = useUsers({ offset: 0, limit: 200, statuses: ['active'] });
  const teams = useOnCallTeams();
  const save = useSaveOnCallSchedule(id);
  const remove = useDeleteOnCallSchedule();
  const now = useMemo(() => Date.now(), []);
  const [draft, setDraft] = useState<OnCallScheduleInput>(
    () => initial ?? { name: '', timeZone: browserTimeZone(), team: null, tags: [], layers: [newLayer(0, now, browserTimeZone())], maxConcurrent: null },
  );
  const [tagText, setTagText] = useState((initial?.tags ?? []).join(', '));
  const [editingName, setEditingName] = useState(false);
  const [layerEdit, setLayerEdit] = useState<{ index: number; layer: OnCallLayer } | null>(null);
  const [viewTz, setViewTz] = useState(draft.timeZone);
  const [frame, setFrame] = useState<Frame>('week');
  const [anchor, setAnchor] = useState(() => frameStart('day', utcToWall(draft.timeZone, now)));
  const [overrideDraft, setOverrideDraft] = useState<{ start: number; end: number; userId: string } | null>(null);
  // New schedules default to "<first name>'s Schedule – <date>" once the profile is known.
  useEffect(() => {
    if (!id && !draft.name && me.data) setDraft((d) => ({ ...d, name: defaultScheduleName(me.data.name, now) }));
  }, [id, draft.name, me.data, now]);

  const from = wallToUtc(viewTz, anchor);
  const to = wallToUtc(viewTz, frameEnd(frame, anchor));
  const preview = useOnCallPreview({ schedule: { ...draft, tags: [] }, scheduleId: id ?? null, range: { from, to } });
  const names = useMemo(() => {
    const m = new Map<string, string>();
    for (const u of users.data?.users ?? []) m.set(u.id, u.name);
    for (const u of preview.data?.users ?? []) m.set(u.id, u.name);
    return m;
  }, [users.data, preview.data]);

  const setLayers = (layers: OnCallLayer[]) => setDraft((d) => ({ ...d, layers }));
  const problems = draft.layers.map(layerProblem).filter(Boolean);
  const submit = () => {
    const tags = tagText
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean);
    save.mutate({ ...draft, tags }, { onSuccess: (s) => navigate(id ? '/on-call/schedules' : `/on-call/schedules/${s.id}`) });
  };
  const columns = frame === 'day' ? Array.from({ length: 8 }, (_, i) => from + i * 3 * HOUR) : dayColumns(viewTz, from, to);
  const nowBlock = timelineBlock(Date.now(), Date.now() + 1, from, to);
  const label = (wall: number) => fmtDay(wall, { month: 'short', day: 'numeric', year: 'numeric' });
  const rangeText = frame === 'day' ? label(anchor) : `${label(anchor)} – ${label(frameEnd(frame, anchor) - DAY)}`;

  return (
    <div className="mon-layout onc-editor">
      <header className="product-header onc-editor-head">
        <div className="product-title onc-crumbs">
          <PhoneCall size={20} />
          <Link to="/on-call">On-Call</Link>
          <ChevronRight size={14} />
          <Link to="/on-call/schedules">Schedules</Link>
          <ChevronRight size={14} />
          {editingName ? (
            <input
              autoFocus
              className="onc-name-input"
              value={draft.name}
              maxLength={200}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              onBlur={() => setEditingName(false)}
              onKeyDown={(e) => e.key === 'Enter' && setEditingName(false)}
            />
          ) : (
            <button className="onc-name" onClick={() => setEditingName(true)} title="Rename">
              {draft.name || 'Untitled schedule'} <Pencil size={13} />
            </button>
          )}
        </div>
        <div className="mon-header-actions">
          {id && (
            <button
              className="btn btn-danger"
              disabled={remove.isPending}
              onClick={() => confirm(`Delete ${draft.name}? Escalation steps that notify it will skip it.`) && remove.mutate(id, { onSuccess: () => navigate('/on-call/schedules') })}
            >
              <Trash2 size={14} /> Delete
            </button>
          )}
          <button className="btn" onClick={() => navigate('/on-call/schedules')}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending || !draft.name.trim() || problems.length > 0} onClick={submit} title={problems[0] ?? undefined}>
            {id ? 'Save' : 'Create'}
          </button>
        </div>
      </header>
      {(save.error || remove.error) && <p className="form-error onc-pad">{errorText(save.error ?? remove.error)}</p>}
      <div className="onc-editor-body">
        <div className="onc-editor-main">
          <div className="onc-toolbar">
            <button className="btn btn-sm" onClick={() => setAnchor(frameStart('day', utcToWall(viewTz, Date.now())))}>
              Today
            </button>
            <button className="icon-btn" title="Previous" onClick={() => setAnchor(stepFrame(frame, anchor, -1))}>
              <ChevronLeft size={16} />
            </button>
            <button className="icon-btn" title="Next" onClick={() => setAnchor(stepFrame(frame, anchor, 1))}>
              <ChevronRight size={16} />
            </button>
            <span className="onc-range-text">{rangeText}</span>
            <span className="onc-toolbar-right">
              <Select label="Time Zone" value={viewTz} onChange={setViewTz} options={timeZoneOptions(viewTz).map((z) => ({ value: z, label: z }))} />
              <Select
                label="Time Frame"
                value={frame}
                onChange={(f) => {
                  setFrame(f);
                  setAnchor(frameStart(f, anchor));
                }}
                options={FRAMES}
              />
            </span>
          </div>
          <div className="onc-grid">
            <div className="onc-grid-row onc-grid-head">
              <div className="onc-grid-label" />
              <div className="onc-track">
                {columns.map((c) => {
                  const b = timelineBlock(c, c + 1, from, to);
                  const wall = utcToWall(viewTz, c);
                  return (
                    b && (
                      <span key={c} className="onc-col-tick" style={{ left: `${b.left}%` }}>
                        {frame === 'day' ? new Date(wall).toISOString().slice(11, 16) : frame === 'month' ? new Date(wall).getUTCDate() : fmtDay(wall, { weekday: 'short', day: 'numeric' })}
                      </span>
                    )
                  );
                })}
              </div>
            </div>
            <div className="onc-grid-section">
              <span className="onc-grid-title">
                <Layers size={16} /> Layers <span className="onc-count">{draft.layers.length}</span>
              </span>
              <button
                className="link-btn"
                disabled={draft.layers.length >= 10}
                onClick={() => setLayerEdit({ index: draft.layers.length, layer: newLayer(draft.layers.length, Date.now(), draft.timeZone) })}
              >
                <Plus size={14} /> Add Layer
              </button>
            </div>
            {draft.layers.map((l, i) => (
              <div key={l.id} className="onc-grid-row">
                <div className="onc-grid-label onc-layer-label">
                  <GripVertical size={14} className="muted" />
                  {(l.members.length === 0 || layerProblem(l)) && (
                    <span title={layerProblem(l) ?? 'No responders: nobody is on call on this layer'}>
                      <AlertTriangle size={14} className="onc-warn" />
                    </span>
                  )}
                  <span className="ellipsis" title={`${l.name} · ${rotationLabel(l.rotationHours)}`}>
                    {l.name}
                  </span>
                  <span className="onc-layer-tools">
                    <button className="icon-btn" title="Edit layer" onClick={() => setLayerEdit({ index: i, layer: structuredClone(l) })}>
                      <Pencil size={12} />
                    </button>
                    <button
                      className="icon-btn"
                      title="Duplicate layer"
                      disabled={draft.layers.length >= 10}
                      onClick={() =>
                        setLayers([...draft.layers.slice(0, i + 1), { ...structuredClone(l), id: `${l.id}-copy-${Date.now().toString(36)}`, name: `${l.name} (copy)` }, ...draft.layers.slice(i + 1)])
                      }
                    >
                      <Copy size={12} />
                    </button>
                    <button className="icon-btn" title="Delete layer" onClick={() => setLayers(draft.layers.filter((_, j) => j !== i))}>
                      <Trash2 size={12} />
                    </button>
                  </span>
                </div>
                <Track from={from} to={to} nowLeft={nowBlock?.left} columns={columns}>
                  {l.members.length === 0 && (
                    <span className="onc-track-warn">
                      <AlertTriangle size={13} /> Select responders for &quot;{l.name}&quot;
                    </span>
                  )}
                  {(preview.data?.layers.find((x) => x.layerId === l.id)?.shifts ?? []).map((s) => {
                    const b = timelineBlock(s.start, s.end, from, to);
                    return b && <Block key={`${s.userId}-${s.start}`} left={b.left} width={b.width} userId={s.userId} name={names.get(s.userId) ?? 'Unknown'} start={s.start} end={s.end} />;
                  })}
                </Track>
              </div>
            ))}
            {draft.layers.length === 0 && <p className="muted onc-pad">Add a layer to rotate responders.</p>}
            <div className="onc-grid-section">
              <span className="onc-grid-title">
                <CalendarDays size={16} /> Final Schedule
              </span>
              <Select
                label="Max concurrent responders"
                value={String(draft.maxConcurrent ?? 0)}
                onChange={(v) => setDraft({ ...draft, maxConcurrent: v === '0' ? null : Number(v) })}
                options={[{ value: '0', label: 'No limit' }, ...[1, 2, 3, 4, 5].map((n) => ({ value: String(n), label: String(n) }))]}
              />
            </div>
            <FinalRow
              data={preview.data}
              from={from}
              to={to}
              columns={columns}
              nowLeft={nowBlock?.left}
              names={names}
              canOverride={!!id}
              onPick={(start, end) => setOverrideDraft({ start, end, userId: me.data?.id ?? '' })}
            />
            {id && <OverrideList scheduleId={id} data={preview.data} />}
          </div>
          {problems.length > 0 && <p className="form-error onc-pad">{problems[0]}</p>}
        </div>
        <aside className="onc-editor-side">
          <Select
            label="Schedule Time Zone"
            value={draft.timeZone}
            onChange={(z) => setDraft({ ...draft, timeZone: z })}
            options={timeZoneOptions(draft.timeZone).map((z) => ({ value: z, label: z }))}
          />
          <p className="muted onc-hint">Handoff and restriction times are in this time zone; handoffs keep their local time across daylight saving changes.</p>
          <Select
            label="Team"
            value={draft.team ?? ''}
            onChange={(t) => setDraft({ ...draft, team: t || null })}
            options={[{ value: '', label: 'No team' }, ...(teams.data ?? []).map((t) => ({ value: t.handle, label: t.name }))]}
          />
          <label className="form-label">
            Tags
            <input value={tagText} onChange={(e) => setTagText(e.target.value)} placeholder="service:order-service, env:prod" />
          </label>
        </aside>
      </div>
      {layerEdit && (
        <LayerModal
          layer={layerEdit.layer}
          users={(users.data?.users ?? []).map((u) => ({ id: u.id, name: u.name }))}
          onClose={() => setLayerEdit(null)}
          onSave={(l) => {
            const next = [...draft.layers];
            next[layerEdit.index] = l;
            setLayers(next);
            setLayerEdit(null);
          }}
        />
      )}
      {overrideDraft && id && (
        <OverrideModal scheduleId={id} initial={overrideDraft} tz={viewTz} users={(users.data?.users ?? []).map((u) => ({ id: u.id, name: u.name }))} onClose={() => setOverrideDraft(null)} />
      )}
    </div>
  );
}

function Track({
  from,
  to,
  nowLeft,
  columns,
  children,
  onMouseDown,
  className = '',
}: {
  from: number;
  to: number;
  nowLeft?: number;
  columns: number[];
  children: ReactNode;
  onMouseDown?: (e: ReactMouseEvent<HTMLDivElement>) => void;
  className?: string;
}) {
  return (
    <div className={`onc-track ${className}`} onMouseDown={onMouseDown}>
      {columns.map((c) => {
        const b = timelineBlock(c, c + 1, from, to);
        return b && <span key={c} className="onc-col-line" style={{ left: `${b.left}%` }} />;
      })}
      {children}
      {nowLeft != null && <span className="onc-now" style={{ left: `${nowLeft}%` }} />}
    </div>
  );
}

function Block({ left, width, userId, name, start, end, override = false }: { left: number; width: number; userId: string; name: string; start: number; end: number; override?: boolean }) {
  return (
    <span
      className={`onc-block c${colorSlot(userId)}${override ? ' override' : ''}`}
      style={{ left: `${left}%`, width: `${width}%` }}
      title={`${name}: ${new Date(start).toLocaleString()} → ${new Date(end).toLocaleString()}${override ? ' (override)' : ''}`}
    >
      {width > 4 && <span className="onc-block-name">{name}</span>}
    </span>
  );
}

function FinalRow({
  data,
  from,
  to,
  columns,
  nowLeft,
  names,
  canOverride,
  onPick,
}: {
  data?: OnCallScheduleShifts;
  from: number;
  to: number;
  columns: number[];
  nowLeft?: number;
  names: Map<string, string>;
  canOverride: boolean;
  onPick: (start: number, end: number) => void;
}) {
  const [drag, setDrag] = useState<{ a: number; b: number } | null>(null);
  const down = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (!canOverride) return;
    const r = e.currentTarget.getBoundingClientRect();
    // Times snap to the hour.
    const at = (clientX: number) => Math.round((from + ((clientX - r.left) / r.width) * (to - from)) / HOUR) * HOUR;
    const a = at(e.clientX);
    setDrag({ a, b: a });
    const move = (ev: MouseEvent) => setDrag({ a, b: at(ev.clientX) });
    const up = (ev: MouseEvent) => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      const b = at(ev.clientX);
      setDrag(null);
      const [s, en] = a === b ? [a, a + 4 * HOUR] : [Math.min(a, b), Math.max(a, b)];
      onPick(s, en);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  const sel = drag && timelineBlock(Math.min(drag.a, drag.b), Math.max(drag.a, drag.b) || drag.a + HOUR, from, to);
  return (
    <div className="onc-grid-row onc-final">
      <div className="onc-grid-label">
        <i className="muted ellipsis">{canOverride ? 'Click or drag to create override' : 'Create the schedule to add overrides'}</i>
      </div>
      <Track from={from} to={to} nowLeft={nowLeft} columns={columns} onMouseDown={down} className={canOverride ? 'pickable' : ''}>
        {(data?.finalShifts ?? []).flatMap((s) => {
          const b = timelineBlock(s.start, s.end, from, to);
          if (!b) return [];
          const h = 100 / s.userIds.length;
          return s.userIds.map((u, i) => (
            <span key={`${s.start}-${u}`} className="onc-final-slot" style={{ top: `${i * h}%`, height: `${h}%`, left: `${b.left}%`, width: `${b.width}%` }}>
              <Block left={0} width={100} userId={u} name={names.get(u) ?? 'Unknown'} start={s.start} end={s.end} override={s.isOverride} />
            </span>
          ));
        })}
        {sel && <span className="onc-select" style={{ left: `${sel.left}%`, width: `${Math.max(sel.width, 0.5)}%` }} />}
      </Track>
    </div>
  );
}

function OverrideList({ scheduleId, data }: { scheduleId: string; data?: OnCallScheduleShifts }) {
  const del = useDeleteOnCallOverride(scheduleId);
  if (!data?.overrides.length) return null;
  return (
    <div className="onc-overrides">
      <h4>Overrides in view</h4>
      {data.overrides.map((o) => (
        <div key={o.id} className="onc-override-row">
          <span className={`onc-swatch c${colorSlot(o.userId)}`} />
          <b>{o.userName}</b>
          <span className="muted">
            {new Date(o.start).toLocaleString()} → {new Date(o.end).toLocaleString()}
          </span>
          <button className="icon-btn" title="Delete override" disabled={del.isPending} onClick={() => del.mutate(o.id)}>
            <Trash2 size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}

/** "YYYY-MM-DDTHH:MM" in tz for a datetime-local input, and back. */
const toInput = (tz: string, ms: number) => formatLocal(utcToWall(tz, ms));
const fromInput = (tz: string, v: string) => wallToUtc(tz, parseLocal(v));

function OverrideModal({
  scheduleId,
  initial,
  tz,
  users,
  onClose,
}: {
  scheduleId: string;
  initial: { start: number; end: number; userId: string };
  tz: string;
  users: { id: string; name: string }[];
  onClose: () => void;
}) {
  const create = useCreateOnCallOverride(scheduleId);
  const [userId, setUserId] = useState(initial.userId);
  const [start, setStart] = useState(toInput(tz, initial.start));
  const [end, setEnd] = useState(toInput(tz, initial.end));
  const s = fromInput(tz, start);
  const e = fromInput(tz, end);
  const ok = userId && Number.isFinite(s) && Number.isFinite(e) && e > s;
  return (
    <Modal onClose={onClose} label="Create override" width={520} height="auto">
      <form className="small-modal onc-modal" onSubmit={(ev) => (ev.preventDefault(), create.mutate({ userId, start: s, end: e }, { onSuccess: onClose }))}>
        <div className="modal-title-row">
          <h3>Create override</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <Select label="Who covers" value={userId} onChange={setUserId} options={[{ value: '', label: 'Select a person…' }, ...users.map((u) => ({ value: u.id, label: u.name }))]} />
        <div className="onc-form-grid">
          <label className="form-label">
            From ({tz})
            <input type="datetime-local" value={start} onChange={(ev) => setStart(ev.target.value)} />
          </label>
          <label className="form-label">
            To ({tz})
            <input type="datetime-local" value={end} onChange={(ev) => setEnd(ev.target.value)} />
          </label>
        </div>
        {create.error && <p className="form-error">{errorText(create.error)}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ok || create.isPending}>
            Create Override
          </button>
        </div>
      </form>
    </Modal>
  );
}

function LayerModal({ layer, users, onClose, onSave }: { layer: OnCallLayer; users: { id: string; name: string }[]; onClose: () => void; onSave: (l: OnCallLayer) => void }) {
  const [l, setL] = useState<OnCallLayer>({ ...layer, restrictions: layer.restrictions ?? [] });
  const [adding, setAdding] = useState('');
  const kind = l.rotationHours === 24 ? 'daily' : l.rotationHours === 168 ? 'weekly' : 'custom';
  const [date, time] = (l.effectiveFrom || 'T').split('T');
  const until = l.effectiveUntil ? l.effectiveUntil.split('T')[0] : '';
  const nameOf = (id: string) => users.find((u) => u.id === id)?.name ?? id;
  const move = (i: number, d: -1 | 1) => {
    const m = [...l.members];
    [m[i], m[i + d]] = [m[i + d], m[i]];
    setL({ ...l, members: m });
  };
  const setRestriction = (i: number, r: Partial<OnCallRestriction>) => setL({ ...l, restrictions: (l.restrictions ?? []).map((x, j) => (j === i ? { ...x, ...r } : x)) });
  const problem = layerProblem(l);
  const dayOptions = WEEKDAYS.map((d) => ({ value: d, label: DAY_LABEL[d] }));
  return (
    <Modal onClose={onClose} label="Edit layer" width={640} height="auto">
      <form className="small-modal onc-modal onc-layer-modal" onSubmit={(e) => (e.preventDefault(), !problem && onSave(l))}>
        <div className="modal-title-row">
          <h3>{layer.members.length ? 'Edit layer' : 'Layer'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={l.name} maxLength={200} onChange={(e) => setL({ ...l, name: e.target.value })} required />
        </label>
        <div className="form-label">
          Responders, in rotation order
          <ol className="onc-member-list">
            {l.members.map((m, i) => (
              <li key={`${m}-${i}`}>
                <span className={`onc-swatch c${colorSlot(m)}`} />
                <span className="ellipsis">{nameOf(m)}</span>
                <button type="button" className="icon-btn" title="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                  <ArrowUp size={12} />
                </button>
                <button type="button" className="icon-btn" title="Move down" disabled={i === l.members.length - 1} onClick={() => move(i, 1)}>
                  <ArrowDown size={12} />
                </button>
                <button type="button" className="icon-btn" title="Remove" onClick={() => setL({ ...l, members: l.members.filter((_, j) => j !== i) })}>
                  <X size={12} />
                </button>
              </li>
            ))}
          </ol>
          <div className="onc-inline">
            <Select value={adding} onChange={setAdding} options={[{ value: '', label: 'Add a responder…' }, ...users.map((u) => ({ value: u.id, label: u.name }))]} />
            <button
              type="button"
              className="btn btn-sm"
              disabled={!adding}
              onClick={() => {
                setL({ ...l, members: [...l.members, adding] });
                setAdding('');
              }}
            >
              <Plus size={13} /> Add
            </button>
          </div>
        </div>
        <div className="onc-form-grid">
          <Select
            label="Rotation"
            value={kind}
            onChange={(k) => setL({ ...l, rotationHours: k === 'daily' ? 24 : k === 'weekly' ? 168 : 12 })}
            options={[
              { value: 'daily', label: 'Daily' },
              { value: 'weekly', label: 'Weekly' },
              { value: 'custom', label: 'Custom' },
            ]}
          />
          {kind === 'custom' && (
            <label className="form-label">
              Shift length (hours)
              <input type="number" min={1} max={8760} value={l.rotationHours} onChange={(e) => setL({ ...l, rotationHours: Number(e.target.value) })} />
            </label>
          )}
          <label className="form-label">
            Start date
            <input type="date" value={date} onChange={(e) => setL({ ...l, effectiveFrom: `${e.target.value}T${time || '09:00'}` })} required />
          </label>
          <label className="form-label">
            Handoff time
            <input type="time" value={time} onChange={(e) => setL({ ...l, effectiveFrom: `${date}T${e.target.value}` })} required />
          </label>
          <label className="form-label">
            End date (optional)
            <input type="date" value={until} onChange={(e) => setL({ ...l, effectiveUntil: e.target.value ? `${e.target.value}T${time || '00:00'}` : null })} />
          </label>
        </div>
        <div className="form-label">
          Restrictions (only on call inside these windows)
          {(l.restrictions ?? []).map((r, i) => (
            <div key={i} className="onc-restriction">
              <Select value={r.startDay} onChange={(v) => setRestriction(i, { startDay: v })} options={dayOptions} />
              <input type="time" value={r.startTime} onChange={(e) => setRestriction(i, { startTime: e.target.value })} />
              <span className="muted">to</span>
              <Select value={r.endDay} onChange={(v) => setRestriction(i, { endDay: v })} options={dayOptions} />
              <input type="time" value={r.endTime} onChange={(e) => setRestriction(i, { endTime: e.target.value })} />
              <button type="button" className="icon-btn" title="Remove restriction" onClick={() => setL({ ...l, restrictions: (l.restrictions ?? []).filter((_, j) => j !== i) })}>
                <Trash2 size={12} />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="link-btn onc-add-restriction"
            disabled={(l.restrictions ?? []).length >= 14}
            onClick={() => setL({ ...l, restrictions: [...(l.restrictions ?? []), { startDay: 'mon', startTime: '09:00', endDay: 'fri', endTime: '17:00' }] })}
          >
            <Plus size={13} /> Add restriction
          </button>
        </div>
        {problem && <p className="form-error">{problem}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!!problem}>
            Save Layer
          </button>
        </div>
      </form>
    </Modal>
  );
}

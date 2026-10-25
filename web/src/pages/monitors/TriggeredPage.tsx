import { useDeferredValue, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Clock, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { MonitorStatus } from '../../api/monitors/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { Facet, FacetCheck, MutedLeft, MuteMenu, PRIORITY_LABEL, StatusBadge, TagChips, ValuesFacet, fmtCount, fmtDateTime, fmtDuration, toggle } from './common';
import { useBulk, useTriggered } from './queries';

const STATUSES: { id: MonitorStatus; label: string; color: string }[] = [
  { id: 'alert', label: 'Alert', color: 'var(--mon-alert)' },
  { id: 'warn', label: 'Warn', color: 'var(--mon-warn)' },
  { id: 'nodata', label: 'No Data', color: 'var(--mon-nodata)' },
];
const WITHIN = [
  { minutes: 30, label: '30min' },
  { minutes: 60, label: '1hr' },
  { minutes: 720, label: '12hr' },
  { minutes: 1440, label: '24hr' },
];
const PAGE = 50;

/** Monitor groups currently in Alert, Warn or No Data, most recently triggered first. */
export function TriggeredPage() {
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const [statuses, setStatuses] = useState<MonitorStatus[]>([]);
  const [muted, setMuted] = useState<boolean[]>([]);
  const [priorities, setPriorities] = useState<string[]>([]);
  const [drafts, setDrafts] = useState<boolean[]>([]);
  const [within, setWithin] = useState<number | null>(null);
  const [custom, setCustom] = useState('');
  const [offset, setOffset] = useState(0);
  const [controls, setControls] = useState(true);
  const [selected, setSelected] = useState<string[]>([]);
  const deferred = useDeferredValue(query);
  const customMinutes = Number(custom) > 0 ? Number(custom) : null;
  const q = useTriggered({
    query: deferred,
    statuses,
    muted,
    priorities: priorities.map(Number),
    drafts,
    triggeredWithin: customMinutes ?? within,
    offset,
    limit: PAGE,
  });
  const bulk = useBulk();
  useEffect(() => setOffset(0), [deferred, statuses, muted, priorities, drafts, within, customMinutes]);
  useEffect(() => setSelected([]), [q.data]);

  const f = q.data?.facets;
  const count = (list: { value: string; count: number }[] | undefined, v: string) => list?.find((x) => x.value === v)?.count ?? 0;
  const multi = <T,>(sel: T[], set: (v: T[]) => void, all: T[], v: T) => {
    const base = sel.length === 0 ? all : sel;
    const next = toggle(base, v);
    set(next.length === all.length ? [] : next);
  };
  const keyOf = (g: { monitorId: number; group: string }) => `${g.monitorId}|${g.group}`;
  const rows = q.data?.groups ?? [];
  const allSelected = rows.length > 0 && rows.every((r) => selected.includes(keyOf(r)));
  const selectedMonitors = [...new Set(selected.map((k) => Number(k.split('|')[0])))];

  return (
    <div className="mon-page">
      <div className="mon-filter">
        <Search size={16} />
        <input placeholder="Filter monitors" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="mon-body">
        {controls && (
          <aside className="mon-controls">
            <Facet title="Status">
              {STATUSES.map((s) => (
                <FacetCheck
                  key={s.id}
                  checked={statuses.length === 0 || statuses.includes(s.id)}
                  onChange={() =>
                    multi(
                      statuses,
                      setStatuses,
                      STATUSES.map((x) => x.id),
                      s.id,
                    )
                  }
                  label={s.label}
                  color={s.color}
                  count={count(f?.status, s.id)}
                />
              ))}
            </Facet>
            <Facet title="Muted">
              {[true, false].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={muted.length === 0 || muted.includes(v)}
                  onChange={() => multi(muted, setMuted, [true, false], v)}
                  label={v ? 'True' : 'False'}
                  count={count(f?.muted, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Triggered Within…">
              {WITHIN.map((w) => (
                <FacetCheck
                  key={w.minutes}
                  checked={!customMinutes && (within === null || w.minutes >= within)}
                  onChange={() => {
                    setCustom('');
                    setWithin(within === w.minutes ? null : w.minutes);
                  }}
                  label={w.label}
                  count={count(f?.triggeredWithin, String(w.minutes))}
                />
              ))}
              <label className="mon-check">
                <input type="checkbox" className="checkbox" checked={!!customMinutes} readOnly />
                <input className="mon-num" type="number" min={0} placeholder="0" value={custom} onChange={(e) => setCustom(e.target.value)} aria-label="Triggered within minutes" /> min
              </label>
            </Facet>
            <Facet title="Draft Status">
              {[false, true].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={drafts.length === 0 || drafts.includes(v)}
                  onChange={() => multi(drafts, setDrafts, [false, true], v)}
                  label={v ? 'draft' : 'published'}
                  count={count(f?.draft, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Priority">
              <ValuesFacet
                values={[...(f?.priority ?? [])].sort((a, b) => (a.value === '0' ? 9 : +a.value) - (b.value === '0' ? 9 : +b.value))}
                selected={priorities}
                onChange={setPriorities}
                label={(v) => PRIORITY_LABEL[v] ?? v}
              />
            </Facet>
          </aside>
        )}
        <main className="mon-main">
          <div className="mon-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="mon-sep" />
            {q.data && (
              <span>
                Showing{' '}
                <b>
                  {q.data.total === 0 ? 0 : offset + 1}–{offset + rows.length}
                </b>{' '}
                of <b>{fmtCount(q.data.total)}</b>
              </span>
            )}
          </div>
          {q.isPending ? (
            <Loading label="Loading triggered monitors…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table">
                <thead>
                  <tr>
                    <th className="col-check">
                      <input type="checkbox" className="checkbox" checked={allSelected} onChange={() => setSelected(allSelected ? [] : rows.map(keyOf))} aria-label="Select all" />
                    </th>
                    <th className="col-prio">PRIORITY</th>
                    <th className="col-status">STATUS</th>
                    <th className="col-muted">MUTED LEFT</th>
                    <th>NAME</th>
                    <th className="col-group">GROUP</th>
                    <th className="col-when">TRIGGERED</th>
                  </tr>
                  <tr className="mon-bulk">
                    <th colSpan={7}>
                      <MuteMenu
                        disabled={!selected.length}
                        onMute={(m) =>
                          bulk.mutate({
                            ids: selectedMonitors,
                            action: 'mute',
                            muteMinutes: m,
                          })
                        }
                        onUnmute={() =>
                          bulk.mutate({
                            ids: selectedMonitors,
                            action: 'unmute',
                          })
                        }
                      />
                      <button
                        className="btn"
                        disabled={!selected.length}
                        onClick={() =>
                          bulk.mutate({
                            ids: selectedMonitors,
                            action: 'resolve',
                          })
                        }
                      >
                        Resolve
                      </button>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((g) => (
                    <tr key={keyOf(g)} className={selected.includes(keyOf(g)) ? 'selected' : ''} onClick={() => navigate(`/monitors/${g.monitorId}`)}>
                      <td className="col-check" onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" className="checkbox" checked={selected.includes(keyOf(g))} onChange={() => setSelected(toggle(selected, keyOf(g)))} aria-label="Select group" />
                      </td>
                      <td className="col-prio">{g.priority ? `P${g.priority}` : '-'}</td>
                      <td className="col-status">
                        <StatusBadge status={g.status} />
                      </td>
                      <td className="col-muted">
                        <MutedLeft muted={g.muted} until={g.mutedUntil} />
                      </td>
                      <td className="mon-name">
                        {g.draft && <span className="mon-draft">DRAFT</span>}
                        <Link to={`/monitors/${g.monitorId}`} onClick={(e) => e.stopPropagation()}>
                          {g.name}
                        </Link>
                      </td>
                      <td className="col-group">{g.group === '*' ? <span className="mon-tag">*</span> : <TagChips tags={g.group.split(',')} max={2} />}</td>
                      <td className="col-when" title={fmtDateTime(g.triggeredAt)}>
                        <Clock size={15} /> {fmtDuration(Date.now() - g.triggeredAt)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {rows.length === 0 && <p className="mon-empty">Nothing is triggered. 🎉</p>}
              <div className="mon-pager">
                <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
              </div>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

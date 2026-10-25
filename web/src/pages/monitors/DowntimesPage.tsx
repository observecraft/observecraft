import { useDeferredValue, useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { PanelLeftClose, PanelLeftOpen, RotateCcw, Search, Wrench, X } from 'lucide-react';
import type { DowntimeItem } from '../../api/monitors/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { Facet, FacetCheck, ValuesFacet, fmtCount, fmtDateTime, toggle } from './common';
import { useCancelDowntime, useDowntimes } from './queries';
import { ScheduleDowntimeModal } from './ScheduleDowntimeModal';

const PAGE = 30;

export function DowntimesPage() {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState<boolean[]>([true]);
  const [recurring, setRecurring] = useState<boolean[]>([]);
  const [creators, setCreators] = useState<string[]>([]);
  const [offset, setOffset] = useState(0);
  const [controls, setControls] = useState(true);
  const [scheduling, setScheduling] = useState(false);
  const deferred = useDeferredValue(query);
  const q = useDowntimes({
    query: deferred,
    active,
    recurring,
    creators,
    includeCanceled: false,
    offset,
    limit: PAGE,
  });
  const cancel = useCancelDowntime();
  useEffect(() => setOffset(0), [deferred, active, recurring, creators]);

  const f = q.data?.facets;
  const count = (list: { value: string; count: number }[] | undefined, v: string) => list?.find((x) => x.value === v)?.count ?? 0;
  const flip = (sel: boolean[], set: (v: boolean[]) => void, v: boolean) => {
    const base = sel.length === 0 ? [true, false] : sel;
    const next = toggle(base, v);
    set(next.length === 2 ? [] : next);
  };
  const chips = [active.length === 1 && `active:${active[0]}`, recurring.length === 1 && `recurring:${recurring[0]}`].filter(Boolean) as string[];

  return (
    <div className="mon-page">
      <div className="mon-filter">
        {chips.map((c) => (
          <span key={c} className="mon-chip">
            {c.split(':')[0]}:<b>{c.split(':')[1]}</b>
          </span>
        ))}
        <Search size={16} />
        <input placeholder="Filter downtimes (monitor, scope, message)" value={query} onChange={(e) => setQuery(e.target.value)} />
        {(chips.length > 0 || query) && (
          <button
            className="icon-plain"
            title="Clear filters"
            onClick={() => {
              setQuery('');
              setActive([]);
              setRecurring([]);
            }}
          >
            <X size={18} />
          </button>
        )}
      </div>
      <div className="mon-body">
        {controls && (
          <aside className="mon-controls">
            <Facet title="Active">
              {[true, false].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={active.length === 0 || active.includes(v)}
                  onChange={() => flip(active, setActive, v)}
                  label={v ? 'True' : 'False'}
                  count={count(f?.active, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Recurring">
              {[false, true].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={recurring.length === 0 || recurring.includes(v)}
                  onChange={() => flip(recurring, setRecurring, v)}
                  label={v ? 'True' : 'False'}
                  count={count(f?.recurring, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Creator">
              <ValuesFacet
                values={(f?.creators ?? []).map((c) => ({
                  value: `${c.value}|${c.label}`,
                  count: c.count,
                }))}
                selected={creators.map((c) => `${c}|${f?.creators.find((x) => x.value === c)?.label ?? ''}`)}
                onChange={(v) => setCreators(v.map((x) => x.split('|')[0]))}
                label={(v) => v.split('|')[1]}
                searchable
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
                  {q.data.total === 0 ? 0 : offset + 1}–{offset + q.data.downtimes.length}
                </b>{' '}
                of <b>{fmtCount(q.data.total)}</b>
              </span>
            )}
            <span className="mon-toolbar-right">
              <button className="btn mon-schedule-btn" onClick={() => setScheduling(true)}>
                <Wrench size={15} /> Schedule Downtime
              </button>
            </span>
          </div>
          {q.isPending ? (
            <Loading label="Loading downtimes…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table mon-dt-table">
                <thead>
                  <tr>
                    <th>STATUS</th>
                    <th>RECURRING</th>
                    <th>SCOPE</th>
                    <th>MONITOR TAGS</th>
                    <th>MONITOR</th>
                    <th>DESCRIPTION</th>
                    <th>START</th>
                    <th>END</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {q.data.downtimes.map((d) => (
                    <Row key={d.id} d={d} onCancel={() => window.confirm('Cancel this downtime? Notifications resume immediately.') && cancel.mutate(d.id)} />
                  ))}
                </tbody>
              </table>
              {q.data.downtimes.length === 0 && <p className="mon-empty">No downtimes match these filters.</p>}
              <div className="mon-pager">
                <Pager offset={offset} limit={PAGE} total={q.data.total} onChange={setOffset} />
              </div>
            </div>
          )}
        </main>
      </div>
      {scheduling && <ScheduleDowntimeModal onClose={() => setScheduling(false)} />}
    </div>
  );
}

function Row({ d, onCancel }: { d: DowntimeItem; onCancel: () => void }) {
  const end = d.status === 'active' && d.recurrence ? d.activeUntil : d.end;
  return (
    <tr>
      <td>
        <span className={`mon-dt-status dt-${d.status}`}>{d.status.toUpperCase()}</span>
      </td>
      <td>
        {d.recurrence && (
          <span className="mon-recurring" title={`Repeats ${d.recurrence}${d.recurUntil ? ` until ${fmtDateTime(d.recurUntil)}` : ''}`}>
            <RotateCcw size={13} /> RECURRING
          </span>
        )}
      </td>
      <td>
        {d.scope.map((s) => (
          <span key={s} className="mon-tag">
            {s}
          </span>
        ))}
      </td>
      <td>
        {d.monitorTags.map((t) => (
          <span key={t} className="mon-tag">
            {t}
          </span>
        ))}
      </td>
      <td className="mon-ellipsis">
        {d.monitorId ? <Link to={`/monitors/${d.monitorId}`}>{d.monitorName}</Link> : <span className="muted">{d.monitorTags.length ? 'Tagged monitors' : 'All monitors'}</span>}
      </td>
      <td className="mon-ellipsis" title={d.message}>
        {d.message}
      </td>
      <td className="mon-nowrap">{fmtDateTime(d.status === 'active' && d.recurrence && d.activeUntil ? d.activeUntil - ((d.end ?? 0) - d.start) : d.start)}</td>
      <td className="mon-nowrap">{end ? fmtDateTime(end) : <span className="muted">No end</span>}</td>
      <td>
        {(d.status === 'active' || d.status === 'scheduled') && (
          <button className="icon-plain" title="Cancel downtime" onClick={onCancel}>
            <X size={16} />
          </button>
        )}
      </td>
    </tr>
  );
}

import { useDeferredValue, useEffect, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { Code2, PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { MonitorSearchRequest, MonitorStatus, MonitorType } from '../../api/monitors/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { Facet, FacetCheck, MutedLeft, MuteMenu, PRIORITY_LABEL, RecipientIcon, RecipientIcons, StatusBadge, TagChips, ValuesFacet, fmtCount, recipientLabel, toggle } from './common';
import { useBulk, useMonitors } from './queries';

const TRIGGERED: MonitorStatus[] = ['alert', 'warn', 'nodata'];
const STATUS_COLOR: Record<MonitorStatus, string> = {
  alert: 'var(--mon-alert)',
  warn: 'var(--mon-warn)',
  nodata: 'var(--mon-nodata)',
  ok: 'var(--mon-ok)',
};

export function MonitorListPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const [statuses, setStatuses] = useState<MonitorStatus[]>([]);
  const [muted, setMuted] = useState<boolean[]>([]);
  const [priorities, setPriorities] = useState<string[]>([]);
  const [types, setTypes] = useState<string[]>([]);
  const [drafts, setDrafts] = useState<boolean[]>([]);
  const [recipients, setRecipients] = useState<string[]>([]);
  const [creators, setCreators] = useState<string[]>([]);
  const [teams, setTeams] = useState<string[]>([]);
  const [sort, setSort] = useState('status');
  const [limit, setLimit] = useState(50);
  const [offset, setOffset] = useState(0);
  const [controls, setControls] = useState(true);
  const [selected, setSelected] = useState<number[]>([]);
  const deferred = useDeferredValue(query);

  const req: MonitorSearchRequest = {
    query: deferred,
    statuses,
    muted,
    priorities: priorities.map(Number),
    types: types as MonitorType[],
    drafts,
    recipients,
    creators: creators.map((c) => c.split('|')[0]),
    teams,
    sort,
    offset,
    limit,
  };
  const q = useMonitors(req);
  const bulk = useBulk();
  useEffect(() => setOffset(0), [deferred, statuses, muted, priorities, types, drafts, recipients, creators, teams, sort, limit]);
  useEffect(() => setSelected([]), [q.data]);

  const f = q.data?.facets;
  const count = (list: { value: string; count: number }[] | undefined, v: string) => list?.find((x) => x.value === v)?.count ?? 0;
  const statusOn = (s: MonitorStatus) => statuses.length === 0 || statuses.includes(s);
  const flipStatus = (list: MonitorStatus[]) => {
    const base: MonitorStatus[] = statuses.length === 0 ? ['alert', 'warn', 'nodata', 'ok'] : statuses;
    const allOn = list.every((s) => base.includes(s));
    const next = allOn ? base.filter((s) => !list.includes(s)) : [...new Set([...base, ...list])];
    setStatuses(next.length === 4 ? [] : next);
  };
  const boolFacet = (sel: boolean[], set: (v: boolean[]) => void, v: boolean) => {
    const base = sel.length === 0 ? [true, false] : sel;
    const next = toggle(base, v);
    set(next.length === 2 ? [] : next);
  };
  const ids = q.data?.monitors.map((m) => m.id) ?? [];
  const allSelected = ids.length > 0 && ids.every((id) => selected.includes(id));

  return (
    <div className="mon-page">
      <div className="mon-filter">
        <Search size={16} />
        <input
          placeholder="Filter monitors (name, tag or key:value)"
          value={query}
          onChange={(e) =>
            setParams(e.target.value ? { q: e.target.value } : {}, {
              replace: true,
            })
          }
        />
        <button className="mon-filter-code" title="Words match name, query and tags; key:value matches tags exactly">
          <Code2 size={16} />
        </button>
      </div>
      <div className="mon-body">
        {controls && (
          <aside className="mon-controls">
            <Facet title="Teams" defaultOpen={false}>
              <ValuesFacet values={f?.teams ?? []} selected={teams} onChange={setTeams} />
            </Facet>
            <Facet title="Status">
              <FacetCheck checked={TRIGGERED.every(statusOn)} onChange={() => flipStatus(TRIGGERED)} label="Triggered" count={TRIGGERED.reduce((n, s) => n + count(f?.status, s), 0)} />
              {TRIGGERED.map((s) => (
                <FacetCheck
                  key={s}
                  indent
                  checked={statusOn(s)}
                  onChange={() => flipStatus([s])}
                  label={{ alert: 'Alert', warn: 'Warn', nodata: 'No Data' }[s as string]}
                  color={STATUS_COLOR[s]}
                  count={count(f?.status, s)}
                />
              ))}
              <FacetCheck checked={statusOn('ok')} onChange={() => flipStatus(['ok'])} label="OK" color={STATUS_COLOR.ok} count={count(f?.status, 'ok')} />
            </Facet>
            <Facet title="Muted">
              {[true, false].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={muted.length === 0 || muted.includes(v)}
                  onChange={() => boolFacet(muted, setMuted, v)}
                  label={v ? 'True' : 'False'}
                  count={count(f?.muted, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Notification Recipient" badge="NEW">
              <ValuesFacet values={f?.recipients ?? []} selected={recipients} onChange={setRecipients} label={recipientLabel} icon={(r) => <RecipientIcon r={r} size={13} />} searchable limit={6} />
            </Facet>
            <Facet title="Priority">
              <ValuesFacet
                values={[...(f?.priority ?? [])].sort((a, b) => (a.value === '0' ? 9 : +a.value) - (b.value === '0' ? 9 : +b.value))}
                selected={priorities}
                onChange={setPriorities}
                label={(v) => PRIORITY_LABEL[v] ?? v}
              />
            </Facet>
            <Facet title="Monitor Type">
              <ValuesFacet values={f?.type ?? []} selected={types} onChange={setTypes} label={(v) => (v === 'apm' ? 'APM' : v === 'log' ? 'Logs' : 'Metric')} />
            </Facet>
            <Facet title="Draft Status">
              {[false, true].map((v) => (
                <FacetCheck
                  key={String(v)}
                  checked={drafts.length === 0 || drafts.includes(v)}
                  onChange={() => boolFacet(drafts, setDrafts, v)}
                  label={v ? 'draft' : 'published'}
                  count={count(f?.draft, String(v))}
                />
              ))}
            </Facet>
            <Facet title="Creator" defaultOpen={false}>
              <ValuesFacet values={f?.creators ?? []} selected={creators} onChange={setCreators} label={(v) => v.split('|')[1] ?? v} searchable />
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
                  {q.data.total === 0 ? 0 : offset + 1}–{offset + q.data.monitors.length}
                </b>{' '}
                of <b>{fmtCount(q.data.total)}</b>
              </span>
            )}
            <span className="mon-toolbar-right">
              <Select
                label="Sort by"
                value={sort}
                onChange={setSort}
                options={[
                  { value: 'status', label: 'Status' },
                  { value: 'name', label: 'Name' },
                  { value: 'priority', label: 'Priority' },
                  { value: 'modified', label: 'Last modified' },
                ]}
              />
            </span>
          </div>
          {q.isPending ? (
            <Loading label="Loading monitors…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table">
                <thead>
                  <tr>
                    <th className="col-check">
                      <input type="checkbox" className="checkbox" checked={allSelected} onChange={() => setSelected(allSelected ? [] : ids)} aria-label="Select all" />
                    </th>
                    <th className="col-prio">PRIORITY</th>
                    <th className="col-status">STATUS</th>
                    <th className="col-muted">MUTED LEFT</th>
                    <th>NAME</th>
                    <th className="col-tags">TAGS</th>
                    <th className="col-rcpt" />
                  </tr>
                  <tr className="mon-bulk">
                    <th colSpan={7}>
                      <MuteMenu
                        disabled={!selected.length}
                        onMute={(m) =>
                          bulk.mutate({
                            ids: selected,
                            action: 'mute',
                            muteMinutes: m,
                          })
                        }
                        onUnmute={() => bulk.mutate({ ids: selected, action: 'unmute' })}
                      />
                      <button className="btn" disabled={!selected.length} onClick={() => bulk.mutate({ ids: selected, action: 'resolve' })}>
                        Resolve
                      </button>
                      <button
                        className="btn"
                        disabled={!selected.length}
                        onClick={() =>
                          window.confirm(`Delete ${selected.length} monitor${selected.length === 1 ? '' : 's'}? This can't be undone.`) && bulk.mutate({ ids: selected, action: 'delete' })
                        }
                      >
                        Delete
                      </button>
                      {['Edit Tags', 'Edit Recipients', 'Edit Teams', 'Export to Terraform'].map((l) => (
                        <button key={l} className="btn" disabled title="Not available yet">
                          {l}
                        </button>
                      ))}
                      {bulk.isError && <span className="form-error">{(bulk.error as Error).message}</span>}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.monitors.map((m) => (
                    <tr key={m.id} className={selected.includes(m.id) ? 'selected' : ''} onClick={() => navigate(`/monitors/${m.id}`)}>
                      <td className="col-check" onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" className="checkbox" checked={selected.includes(m.id)} onChange={() => setSelected(toggle(selected, m.id))} aria-label={`Select ${m.name}`} />
                      </td>
                      <td className="col-prio">{m.priority ? `P${m.priority}` : '-'}</td>
                      <td className="col-status">
                        <StatusBadge status={m.status} />
                      </td>
                      <td className="col-muted">
                        <MutedLeft muted={m.muted} until={m.mutedUntil} />
                      </td>
                      <td className="mon-name">
                        {m.draft && <span className="mon-draft">DRAFT</span>}
                        <Link to={`/monitors/${m.id}`} onClick={(e) => e.stopPropagation()} title={m.query}>
                          {m.name}
                        </Link>
                      </td>
                      <td className="col-tags">
                        <TagChips tags={m.tags} max={2} />
                      </td>
                      <td className="col-rcpt">
                        <RecipientIcons recipients={m.recipients} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {q.data.monitors.length === 0 && <p className="mon-empty">No monitors match these filters.</p>}
              <div className="mon-pager">
                <span>Results per page:</span>
                <Select
                  value={String(limit)}
                  onChange={(v) => setLimit(Number(v))}
                  options={['25', '50', '100'].map((v) => ({
                    value: v,
                    label: v,
                  }))}
                />
                <Pager offset={offset} limit={limit} total={q.data.total} onChange={setOffset} />
              </div>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

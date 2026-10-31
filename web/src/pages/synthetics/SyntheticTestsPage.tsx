import { useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Download, FolderPlus, MoreVertical, PanelLeftClose, PanelLeftOpen, Pause, Pencil, Play, Plus, Radar, Search, Settings2, Trash2, Users, Zap } from 'lucide-react';
import type { SyntheticTest, SyntheticTestType } from '../../api/synthetics/client';
import { TEST_TYPES, TYPE_LABEL, draftOf, fmtFrequency, fmtUptime } from '../../api/synthetics/logic';
import { matchesTestTerms, toCsv } from '../../api/synthetics/model';
import type { Facet } from '../../api/types';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { formatQuery, parseQuery, toggleValue } from '../../lib/searchQuery';
import { STATUS_LABEL, TagChips } from '../monitors/common';
import { TestStatus, TypeIcon } from './common';
import { useDeleteSyntheticTests, useRunSyntheticTests, useSyntheticTests, useUpdateSyntheticTests } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';

type Column = 'domain' | 'tags' | 'envs' | 'frequency' | 'uptime';
const COLUMN_LABEL: Record<Column, string> = { domain: 'Domain', tags: 'Tags', envs: 'Envs', frequency: 'Frequency', uptime: 'Uptime' };
type BulkEdit = 'tags' | 'env' | 'team';

const minute = 60_000;

function count<T>(items: T[], key: (t: T) => string[]): { value: string; count: number }[] {
  const m = new Map<string, number>();
  for (const it of items) for (const k of key(it)) if (k) m.set(k, (m.get(k) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([value, n]) => ({ value, count: n }));
}

const statusOf = (t: SyntheticTest) => (t.paused ? 'paused' : t.status);
const STATUS_FACET: Record<string, string> = { ...STATUS_LABEL, paused: 'Paused' };

/** Synthetic Monitoring & Testing → Tests: every test of the org with its state and uptime, facets and bulk actions. */
export function SyntheticTestsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const { range, relative } = useTime();
  // Uptime is computed per minute: keep the request stable while live mode slides the window.
  const q = useSyntheticTests({ query: '', range: { from: Math.floor(range.from / minute) * minute, to: Math.floor(range.to / minute) * minute } });
  const [controls, setControls] = useState(true);
  const [hidden, setHidden] = useState<Set<Column>>(new Set(['frequency']));
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<BulkEdit | null>(null);
  const remove = useDeleteSyntheticTests();
  const runNow = useRunSyntheticTests();
  const update = useUpdateSyntheticTests();

  const setQuery = (v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set('q', v);
    else next.delete('q');
    setParams(next, { replace: true });
  };
  const all = useMemo(() => q.data?.tests ?? [], [q.data]);
  const tests = useMemo(() => {
    try {
      return all.filter((t) => matchesTestTerms(t, query));
    } catch {
      return all;
    }
  }, [all, query]);
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo<Facet[]>(() => {
    const keys = [...new Set(all.flatMap((t) => t.tags.filter((x) => x.includes(':')).map((x) => x.slice(0, x.indexOf(':')))))].filter((k) => k !== 'team').sort();
    return [
      { key: 'type', label: 'Type', group: 'SYNTHETIC FILTERS', values: count(all, (t) => [t.type]) },
      { key: 'status', label: 'Status', group: 'SYNTHETIC FILTERS', values: count(all, (t) => [statusOf(t)]) },
      { key: 'creator', label: 'Creator', group: 'SYNTHETIC FILTERS', values: count(all, (t) => [t.createdBy ?? '']) },
      { key: 'env', label: 'Env', group: 'SYNTHETIC FILTERS', values: count(all, (t) => [t.env]) },
      ...keys.map((k) => ({ key: k, label: k, group: 'TAGS', values: count(all, (t) => t.tags.filter((x) => x.startsWith(`${k}:`)).map((x) => x.slice(k.length + 1))) })),
    ];
  }, [all]);
  const teams = useMemo(() => count(all, (t) => [t.team]), [all]);
  const teamFilter = terms.find((t) => t.kind === 'tag' && t.key === 'team' && !t.negate);
  const toggleTeam = (team: string) => setQuery(formatQuery(toggleValue(terms, 'team', team)));
  const filter = (key: string, value: string) => setQuery(formatQuery(toggleValue(terms, key, value)));

  const chosen = tests.filter((t) => selected.has(t.id));
  const allChecked = tests.length > 0 && chosen.length === tests.length;
  const toggleAll = () => setSelected(allChecked ? new Set() : new Set(tests.map((t) => t.id)));
  const toggle = (id: string) =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const show = (c: Column) => !hidden.has(c);

  const download = (rows: SyntheticTest[]) => {
    const csv = toCsv(
      ['STATUS', 'TYPE', 'NAME', 'DOMAIN', 'TAGS', 'ENVS', 'TEAM', 'CREATOR', 'FREQUENCY', 'UPTIME', 'ID'],
      rows.map((t) => [statusOf(t), t.type, t.name, t.domain, t.tags.join(' '), t.env, t.team, t.createdBy ?? '', fmtFrequency(t.frequencySeconds), fmtUptime(t.uptime), t.id]),
    );
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    a.download = 'synthetic-tests.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const removeTests = (rows: SyntheticTest[]) => {
    if (
      !window.confirm(
        `Delete ${rows.length === 1 ? `“${rows[0].name}”` : `${rows.length} tests`}, ${rows.length === 1 ? 'its' : 'their'} results and monitor${rows.length === 1 ? '' : 's'}? This can't be undone.`,
      )
    )
      return;
    remove.mutate(
      rows.map((t) => t.id),
      { onSuccess: () => setSelected(new Set()) },
    );
  };
  const setPaused = (rows: SyntheticTest[], paused: boolean) => update.mutate(rows.map((t) => ({ id: t.id, input: { ...draftOf(t), paused } })));
  const anyRunning = chosen.some((t) => !t.paused);

  const typeCounts = (pred: (t: SyntheticTest) => boolean) => TEST_TYPES.map((type) => [type, all.filter((t) => t.type === type && pred(t)).length] as [SyntheticTestType, number]);
  const isAlert = (t: SyntheticTest) => !t.paused && t.status === 'alert';
  const isOk = (t: SyntheticTest) => !t.paused && t.status === 'ok';
  const alerting = all.filter(isAlert).length;
  const ok = all.filter(isOk).length;
  const busy = remove.isPending || runNow.isPending || update.isPending;
  const error = (remove.error ?? runNow.error ?? update.error) as Error | null;

  return (
    <div className="syn-page syn-explorer">
      <div className="syn-searchbar">
        <Search size={16} />
        <input placeholder="Type to start searching" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search tests" />
        <TimePicker compact />
      </div>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={(t) => setQuery(formatQuery(t))}
            defaultOpen={['type', 'status', 'creator']}
            verified={[]}
            noun="facets"
            valueLabel={(k, v) => (k === 'status' ? (STATUS_FACET[v] ?? v) : k === 'type' ? (TYPE_LABEL[v as SyntheticTestType] ?? v) : v)}
            header={
              <section className="syn-teams-card">
                <h4>
                  <Users size={14} /> Teams
                </h4>
                {teams.length === 0 ? (
                  <p className="muted">No test has a team yet.</p>
                ) : (
                  teams.map((t) => (
                    <label key={t.value} className="syn-team-row">
                      <input type="checkbox" className="checkbox" checked={teamFilter?.kind === 'tag' && teamFilter.values.includes(t.value)} onChange={() => toggleTeam(t.value)} />
                      <span className="syn-grow">{t.value}</span>
                      <span className="muted">{t.count}</span>
                    </label>
                  ))
                )}
              </section>
            }
          />
        )}
        <div className="syn-explorer-main">
          <div className="syn-summary-tiles">
            {(
              [
                ['alert', 'ALERT', alerting, isAlert],
                ['ok', 'OK', ok, isOk],
              ] as const
            ).map(([status, label, n, pred]) => (
              <div key={status} className={`syn-tile syn-tile-types ${status}`}>
                <button className="syn-tile-main" onClick={() => filter('status', status)}>
                  <span>{label}</span>
                  <b>{n}</b> test{n === 1 ? '' : 's'}
                </button>
                <div className="syn-tile-chips">
                  {typeCounts(pred).map(([type, c]) => (
                    <button key={type} className="syn-tile-chip" onClick={() => setQuery(formatQuery(toggleValue(toggleValue(terms, 'status', status), 'type', type)))}>
                      {c} {TYPE_LABEL[type]}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
          <div className="syn-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="syn-toolbar-sep" />
            <span>
              Showing <b>{tests.length}</b> of <b>{all.length}</b> test{all.length === 1 ? '' : 's'}
            </span>
            <span className="syn-grow" />
            {q.data && (
              <span className="muted">
                Running from {q.data.location.name} · uptime over the {relative.label.toLowerCase()}
              </span>
            )}
            <Menu
              trigger={
                <>
                  <Settings2 size={14} /> Options
                </>
              }
              items={(Object.keys(COLUMN_LABEL) as Column[]).map((c) => ({
                label: `${show(c) ? 'Hide' : 'Show'} ${COLUMN_LABEL[c]}`,
                onClick: () =>
                  setHidden((h) => {
                    const next = new Set(h);
                    if (next.has(c)) next.delete(c);
                    else next.add(c);
                    return next;
                  }),
              }))}
            />
          </div>
          {q.isPending ? (
            <Loading label="Loading synthetic tests…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : all.length === 0 ? (
            <div className="syn-empty">
              <Radar size={36} strokeWidth={1.4} />
              <h3>Check your endpoints from the ObserveCraft server</h3>
              <p className="muted">HTTP, SSL, DNS and TCP tests run on a schedule, alert through your notification channels and write synthetics.* metrics for dashboards and SLOs.</p>
              <button className="btn btn-primary" onClick={() => navigate('/synthetics/create')}>
                <Plus size={15} /> New Test
              </button>
            </div>
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table syn-table syn-tests-table">
                <thead>
                  <tr className="syn-bulk-row">
                    <th colSpan={10}>
                      <span className={`syn-bulk${chosen.length ? '' : ' idle'}`}>
                        {chosen.length > 0 && <b>{chosen.length} selected</b>}
                        <button className="btn btn-sm" disabled={!chosen.length || busy} onClick={() => runNow.mutate(chosen.map((t) => t.id))}>
                          <Zap size={13} /> {runNow.isPending ? 'Running…' : 'Run Tests Now'}
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length || busy} onClick={() => setPaused(chosen, anyRunning)}>
                          {anyRunning ? <Pause size={13} /> : <Play size={13} />} {anyRunning ? 'Pause Scheduling' : 'Resume Scheduling'}
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length} onClick={() => navigate(`/synthetics/suites/new?tests=${chosen.map((t) => t.id).join(',')}`)}>
                          <FolderPlus size={13} /> Create Suite
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length} onClick={() => download(chosen)}>
                          <Download size={13} /> Download CSV
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length} onClick={() => setEditing('tags')}>
                          <Pencil size={13} /> Edit Tags
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length} onClick={() => setEditing('env')}>
                          <Pencil size={13} /> Edit Envs
                        </button>
                        <button className="btn btn-sm" disabled={!chosen.length} onClick={() => setEditing('team')}>
                          <Pencil size={13} /> Edit Teams
                        </button>
                        <button className="btn btn-sm syn-danger" disabled={!chosen.length || busy} onClick={() => removeTests(chosen)}>
                          <Trash2 size={13} /> Delete
                        </button>
                      </span>
                    </th>
                  </tr>
                  <tr>
                    <th className="syn-col-check">
                      <input type="checkbox" className="checkbox" checked={allChecked} onChange={toggleAll} aria-label="Select every test" />
                    </th>
                    <th className="syn-col-status">STATUS</th>
                    <th className="syn-col-type">TYPE</th>
                    <th>NAME</th>
                    {show('domain') && <th>DOMAIN</th>}
                    {show('tags') && <th className="col-tags">TAGS</th>}
                    {show('envs') && <th className="syn-col-env">ENVS</th>}
                    {show('frequency') && <th className="syn-col-freq">FREQUENCY</th>}
                    {show('uptime') && <th className="syn-col-uptimebar">UPTIME</th>}
                    <th className="syn-col-menu" />
                  </tr>
                </thead>
                <tbody>
                  {tests.map((t) => (
                    <tr key={t.id} onClick={() => navigate(`/synthetics/tests/${t.id}`)}>
                      <td className="syn-col-check" onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" className="checkbox" checked={selected.has(t.id)} onChange={() => toggle(t.id)} aria-label={`Select ${t.name}`} />
                      </td>
                      <td className="syn-col-status">
                        <TestStatus status={t.status} paused={t.paused} small />
                      </td>
                      <td className="syn-col-type">
                        <span className="syn-icon-text">
                          <TypeIcon type={t.type} /> {TYPE_LABEL[t.type]}
                        </span>
                      </td>
                      <td className="mon-name">
                        <span className="syn-name">{t.name}</span>
                        {t.muted && <span className="syn-muted-tag">muted</span>}
                      </td>
                      {show('domain') && <td className="syn-domain">{t.domain}</td>}
                      {show('tags') && (
                        <td className="col-tags">
                          <TagChips tags={t.tags} max={2} />
                        </td>
                      )}
                      {show('envs') && <td className="syn-col-env">{t.env ? <span className="syn-chip">{t.env}</span> : <span className="muted">—</span>}</td>}
                      {show('frequency') && <td className="syn-col-freq muted">{fmtFrequency(t.frequencySeconds)}</td>}
                      {show('uptime') && (
                        <td className="syn-col-uptimebar">
                          <span className={`syn-uptime-cell${t.uptime != null && t.uptime < 99 ? ' low' : ''}`}>
                            <span>{fmtUptime(t.uptime)}</span>
                            <i className={t.uptime == null ? 'none' : t.uptime < 99 ? 'low' : ''} style={{ ['--up' as string]: `${t.uptime ?? 100}%` }} />
                          </span>
                        </td>
                      )}
                      <td className="syn-col-menu" onClick={(e) => e.stopPropagation()}>
                        <Menu
                          triggerClassName="icon-plain"
                          trigger={<MoreVertical size={16} aria-label={`Actions for ${t.name}`} />}
                          items={[
                            { label: 'Run Test Now', onClick: () => runNow.mutate([t.id]) },
                            { label: t.paused ? 'Resume Scheduling' : 'Pause Scheduling', onClick: () => setPaused([t], !t.paused) },
                            { label: 'Edit', onClick: () => navigate(`/synthetics/tests/${t.id}/edit`) },
                            { label: 'Clone', onClick: () => navigate(`/synthetics/create?clone=${t.id}`) },
                            { label: 'Delete', onClick: () => removeTests([t]) },
                          ]}
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {tests.length === 0 && (
                <div className="syn-empty syn-empty-search">
                  <Search size={56} strokeWidth={1.2} />
                  <h3>No matching tests found</h3>
                  <p className="muted">Change the search or the facets to see more tests.</p>
                </div>
              )}
            </div>
          )}
          {error && <p className="form-error">{error.message}</p>}
        </div>
      </div>
      {editing && <BulkEditDialog kind={editing} tests={chosen} onClose={() => setEditing(null)} onDone={() => setEditing(null)} />}
    </div>
  );
}

const BULK_TITLE: Record<BulkEdit, string> = { tags: 'Edit Tags', env: 'Edit Envs', team: 'Edit Teams' };

/** Edits the tags, environment or team of the selected tests at once. */
function BulkEditDialog({ kind, tests, onClose, onDone }: { kind: BulkEdit; tests: SyntheticTest[]; onClose: () => void; onDone: () => void }) {
  const save = useUpdateSyntheticTests();
  const [add, setAdd] = useState('');
  const [drop, setDrop] = useState('');
  const [value, setValue] = useState(() => (tests.length === 1 ? (kind === 'env' ? tests[0].env : kind === 'team' ? tests[0].team : '') : ''));
  const list = (s: string) =>
    s
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean);
  const apply = () => {
    const changes = tests.map((t) => {
      const input = draftOf(t);
      if (kind === 'tags') input.tags = [...new Set([...input.tags.filter((x) => !list(drop).includes(x)), ...list(add)])];
      if (kind === 'env') input.env = value.trim();
      if (kind === 'team') input.team = value.trim();
      return { id: t.id, input };
    });
    save.mutate(changes, { onSuccess: onDone });
  };
  return (
    <Modal onClose={onClose} label={BULK_TITLE[kind]} width={480} height="auto">
      <div className="syn-dialog">
        <h3>
          {BULK_TITLE[kind]} · {tests.length} test{tests.length === 1 ? '' : 's'}
        </h3>
        {kind === 'tags' ? (
          <>
            <label className="form-label">
              Add tags
              <input className="syn-input" placeholder="tier:critical, service:order-service" value={add} onChange={(e) => setAdd(e.target.value)} autoFocus />
            </label>
            <label className="form-label">
              Remove tags
              <input className="syn-input" placeholder="team:legacy" value={drop} onChange={(e) => setDrop(e.target.value)} />
            </label>
          </>
        ) : (
          <label className="form-label">
            {kind === 'env' ? 'Environment' : 'Team'}
            <input className="syn-input" placeholder={kind === 'env' ? 'prod' : 'checkout'} value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
            <span className="muted">Leave empty to clear it.</span>
          </label>
        )}
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="syn-dialog-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending} onClick={apply}>
            {save.isPending ? 'Saving…' : 'Apply'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

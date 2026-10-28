import { useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Download, PanelLeftClose, PanelLeftOpen, Pencil, Search, Settings2, Trash2, Users } from 'lucide-react';
import type { SyntheticSuite, SyntheticSuiteInput } from '../../api/synthetics/client';
import { fmtUptime } from '../../api/synthetics/logic';
import { matchesSuiteTerms, toCsv } from '../../api/synthetics/model';
import type { Facet } from '../../api/types';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { parseQuery, formatQuery, toggleValue } from '../../lib/searchQuery';
import { STATUS_LABEL, StatusBadge, TagChips } from '../monitors/common';
import { useDeleteSyntheticSuites, useSyntheticSuites, useUpdateSyntheticSuites } from './queries';

type Column = 'tags' | 'envs' | 'uptime' | 'modified';
const COLUMN_LABEL: Record<Column, string> = { tags: 'Tags', envs: 'Envs', uptime: 'Uptime', modified: 'Last modified' };
type BulkEdit = 'tags' | 'env' | 'team';

const minute = 60_000;

function inputOf(s: SyntheticSuite): SyntheticSuiteInput {
  return { name: s.name, description: s.description, env: s.env, team: s.team, tags: [...s.tags], testIds: s.tests.map((t) => t.id) };
}

function count<T>(items: T[], key: (t: T) => string[]): { value: string; count: number }[] {
  const m = new Map<string, number>();
  for (const it of items) for (const k of key(it)) if (k) m.set(k, (m.get(k) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([value, n]) => ({ value, count: n }));
}

/** Synthetic Monitoring & Testing → Test Suites: the org's suites with their state, uptime and bulk actions. */
export function TestSuitesPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const { range, relative } = useTime();
  const q = useSyntheticSuites({ query: '', range: { from: Math.floor(range.from / minute) * minute, to: Math.floor(range.to / minute) * minute } });
  const [controls, setControls] = useState(true);
  const [hidden, setHidden] = useState<Set<Column>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<BulkEdit | null>(null);
  const remove = useDeleteSyntheticSuites();

  const setQuery = (v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set('q', v);
    else next.delete('q');
    setParams(next, { replace: true });
  };
  const all = q.data?.suites ?? [];
  const suites = useMemo(() => {
    try {
      return all.filter((s) => matchesSuiteTerms(s, query));
    } catch {
      return all;
    }
  }, [all, query]);
  const terms = useMemo(() => parseQuery(query), [query]);
  const facets = useMemo<Facet[]>(() => {
    const keys = [...new Set(all.flatMap((s) => s.tags.filter((t) => t.includes(':')).map((t) => t.slice(0, t.indexOf(':')))))].sort();
    return [
      { key: 'status', label: 'Status', group: 'SYNTHETIC FILTERS', values: count(all, (s) => [s.status]) },
      { key: 'env', label: 'Env', group: 'SYNTHETIC FILTERS', values: count(all, (s) => [s.env]) },
      ...keys.map((k) => ({ key: k, label: k, group: 'TAGS', values: count(all, (s) => s.tags.filter((t) => t.startsWith(`${k}:`)).map((t) => t.slice(k.length + 1))) })),
    ];
  }, [all]);
  const teams = useMemo(() => count(all, (s) => [s.team]), [all]);
  const teamFilter = terms.find((t) => t.kind === 'tag' && t.key === 'team' && !t.negate);
  const toggleTeam = (team: string) => setQuery(formatQuery(toggleValue(terms, 'team', team)));
  const filterStatus = (status: string) => setQuery(formatQuery(toggleValue(terms, 'status', status)));

  const chosen = suites.filter((s) => selected.has(s.id));
  const allChecked = suites.length > 0 && chosen.length === suites.length;
  const toggleAll = () => setSelected(allChecked ? new Set() : new Set(suites.map((s) => s.id)));
  const toggle = (id: string) =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const show = (c: Column) => !hidden.has(c);

  const download = (rows: SyntheticSuite[]) => {
    const csv = toCsv(
      ['STATUS', 'NAME', 'TESTS', 'TAGS', 'ENVS', 'TEAM', 'UPTIME', 'LAST MODIFIED'],
      rows.map((s) => [s.status, s.name, s.tests.length, s.tags.join(' '), s.env, s.team, fmtUptime(s.uptime), new Date(s.updatedAt).toISOString()]),
    );
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    a.download = 'synthetic-test-suites.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const removeChosen = () => {
    if (!window.confirm(`Delete ${chosen.length} suite${chosen.length === 1 ? '' : 's'}? Their tests stay.`)) return;
    remove.mutate(
      chosen.map((s) => s.id),
      { onSuccess: () => setSelected(new Set()) },
    );
  };

  const alerting = all.filter((s) => s.status === 'alert').length;
  const ok = all.filter((s) => s.status === 'ok').length;

  return (
    <div className="syn-page syn-explorer">
      <div className="syn-searchbar">
        <Search size={16} />
        <input placeholder="Type to start searching" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search suites" />
        <TimePicker compact />
      </div>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={(t) => setQuery(formatQuery(t))}
            defaultOpen={['status', 'env']}
            verified={[]}
            noun="facets"
            valueLabel={(k, v) => (k === 'status' ? (STATUS_LABEL[v as keyof typeof STATUS_LABEL] ?? v) : v)}
            header={
              <section className="syn-teams-card">
                <h4>
                  <Users size={14} /> Teams
                </h4>
                {teams.length === 0 ? (
                  <p className="muted">No suite has a team yet.</p>
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
            <button className="syn-tile alert" onClick={() => filterStatus('alert')}>
              <span>ALERT</span>
              <b>{alerting}</b> suite{alerting === 1 ? '' : 's'}
            </button>
            <button className="syn-tile ok" onClick={() => filterStatus('ok')}>
              <span>OK</span>
              <b>{ok}</b> suite{ok === 1 ? '' : 's'}
            </button>
          </div>
          <div className="syn-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={15} /> : <PanelLeftOpen size={15} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="syn-toolbar-sep" />
            <span>
              <b>{suites.length}</b> matching suite{suites.length === 1 ? '' : 's'}
            </span>
            <span className="syn-toolbar-sep" />
            <label className="syn-check muted" title="Suites aren't restricted in ObserveCraft: everyone in the organization sees every suite.">
              <input type="checkbox" className="checkbox" disabled /> Show restricted suites
            </label>
            <span className="syn-grow" />
            <span className="muted">Uptime over the {relative.label.toLowerCase()}</span>
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
            <Loading label="Loading test suites…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table syn-table">
                <thead>
                  {chosen.length > 0 && (
                    <tr className="syn-bulk-row">
                      <th colSpan={9}>
                        <span className="syn-bulk">
                          <b>{chosen.length} selected</b>
                          <button className="btn btn-sm" onClick={() => download(chosen)}>
                            <Download size={13} /> Download CSV
                          </button>
                          <button className="btn btn-sm" onClick={() => setEditing('tags')}>
                            <Pencil size={13} /> Edit Tags
                          </button>
                          <button className="btn btn-sm" onClick={() => setEditing('env')}>
                            <Pencil size={13} /> Edit Envs
                          </button>
                          <button className="btn btn-sm" onClick={() => setEditing('team')}>
                            <Pencil size={13} /> Edit Teams
                          </button>
                          <button className="btn btn-sm syn-danger" disabled={remove.isPending} onClick={removeChosen}>
                            <Trash2 size={13} /> Delete
                          </button>
                        </span>
                      </th>
                    </tr>
                  )}
                  <tr>
                    <th className="syn-col-check">
                      <input type="checkbox" className="checkbox" checked={allChecked} onChange={toggleAll} aria-label="Select every suite" />
                    </th>
                    <th className="syn-col-status">STATUS</th>
                    <th>NAME</th>
                    <th className="syn-col-tests">TESTS</th>
                    {show('tags') && <th className="col-tags">TAGS</th>}
                    {show('envs') && <th className="syn-col-env">ENVS</th>}
                    {show('uptime') && <th className="syn-col-uptime">UPTIME</th>}
                    {show('modified') && <th className="syn-col-modified">LAST MODIFIED</th>}
                  </tr>
                </thead>
                <tbody>
                  {suites.map((s) => (
                    <tr key={s.id} onClick={() => navigate(`/synthetics/suites/${s.id}`)}>
                      <td className="syn-col-check" onClick={(e) => e.stopPropagation()}>
                        <input type="checkbox" className="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)} aria-label={`Select ${s.name}`} />
                      </td>
                      <td className="syn-col-status">
                        <StatusBadge status={s.status} small />
                      </td>
                      <td className="mon-name">
                        <span className="syn-name">{s.name}</span>
                        {s.description && <span className="muted syn-sub">{s.description}</span>}
                      </td>
                      <td className="syn-col-tests">
                        <span className="syn-dots" title={s.tests.map((t) => `${t.name}: ${t.paused ? 'paused' : t.status}`).join('\n')}>
                          {s.tests.slice(0, 12).map((t) => (
                            <i key={t.id} className={`syn-dot ${t.paused ? 'paused' : t.status}`} />
                          ))}
                        </span>
                        {s.tests.length}
                      </td>
                      {show('tags') && (
                        <td className="col-tags">
                          <TagChips tags={s.tags} max={2} />
                        </td>
                      )}
                      {show('envs') && <td className="syn-col-env">{s.env ? <span className="syn-chip">{s.env}</span> : <span className="muted">—</span>}</td>}
                      {show('uptime') && <td className={`syn-col-uptime${s.uptime != null && s.uptime < 99 ? ' low' : ''}`}>{fmtUptime(s.uptime)}</td>}
                      {show('modified') && (
                        <td className="syn-col-modified muted" title={new Date(s.updatedAt).toLocaleString()}>
                          {timeAgo(s.updatedAt)}
                          {s.updatedBy ? ` by ${s.updatedBy}` : ''}
                        </td>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
              {suites.length === 0 && (
                <div className="syn-empty syn-empty-search">
                  <Search size={56} strokeWidth={1.2} />
                  <h3>No matching suites found</h3>
                  <p className="muted">{all.length === 0 ? 'Group tests into a suite to follow a journey or a team’s endpoints together.' : 'Change the search or the facets to see more suites.'}</p>
                  {all.length === 0 && (
                    <button className="btn btn-primary" onClick={() => navigate('/synthetics/suites/new')}>
                      New Suite
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
          {remove.isError && <p className="form-error">{(remove.error as Error).message}</p>}
        </div>
      </div>
      {editing && <BulkEditDialog kind={editing} suites={chosen} onClose={() => setEditing(null)} onDone={() => setEditing(null)} />}
    </div>
  );
}

const BULK_TITLE: Record<BulkEdit, string> = { tags: 'Edit Tags', env: 'Edit Envs', team: 'Edit Teams' };

/** Edits the tags, environment or team of the selected suites at once. */
function BulkEditDialog({ kind, suites, onClose, onDone }: { kind: BulkEdit; suites: SyntheticSuite[]; onClose: () => void; onDone: () => void }) {
  const save = useUpdateSyntheticSuites();
  const [add, setAdd] = useState('');
  const [drop, setDrop] = useState('');
  const [value, setValue] = useState(() => (suites.length === 1 ? (kind === 'env' ? suites[0].env : kind === 'team' ? suites[0].team : '') : ''));
  const list = (s: string) =>
    s
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean);
  const apply = () => {
    const changes = suites.map((s) => {
      const input = inputOf(s);
      if (kind === 'tags') input.tags = [...new Set([...(input.tags ?? []).filter((t) => !list(drop).includes(t)), ...list(add)])];
      if (kind === 'env') input.env = value.trim();
      if (kind === 'team') input.team = value.trim();
      return { id: s.id, input };
    });
    save.mutate(changes, { onSuccess: onDone });
  };
  return (
    <Modal onClose={onClose} label={BULK_TITLE[kind]} width={480} height="auto">
      <div className="syn-dialog">
        <h3>
          {BULK_TITLE[kind]} · {suites.length} suite{suites.length === 1 ? '' : 's'}
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

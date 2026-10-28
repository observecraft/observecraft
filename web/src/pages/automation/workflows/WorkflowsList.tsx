import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowDownUp, ChevronRight, Columns3, Plus, Search, SearchX } from 'lucide-react';
import { allSteps, TRIGGER_LABEL } from '../../../api/workflows/logic';
import type { Workflow, WorkflowTriggerKind } from '../../../api/workflows/types';
import { usePopover } from '../../../components/ui/Dropdown';
import { useProfile } from '../../settings/queries';
import { useWorkflows } from '../queries';
import { BLUEPRINTS } from './blueprints';
import { fmtAgo, LogoChip, RunStatusChip, TRIGGER_ICON } from './common';
import { useStartFromBlueprint } from './useStartFromBlueprint';
import './workflows.css';

type SortKey = 'name' | 'modified' | 'executed';
type Column = 'triggers' | 'owner' | 'modified' | 'tags' | 'executed';
const COLUMNS: [Column, string][] = [
  ['triggers', 'Triggers'],
  ['owner', 'Owner'],
  ['modified', 'Last Modified'],
  ['tags', 'Tags'],
  ['executed', 'Last Execution'],
];

type Facets = { team: Set<string>; status: Set<string>; trigger: Set<string>; owner: Set<string>; service: Set<string>; tag: Set<string> };
const emptyFacets = (): Facets => ({ team: new Set(), status: new Set(), trigger: new Set(), owner: new Set(), service: new Set(), tag: new Set() });

function values(w: Workflow, facet: keyof Facets): string[] {
  switch (facet) {
    case 'team':
      return w.ownerTeam ? [w.ownerTeam] : [];
    case 'status':
      return [w.published ? 'Published' : 'Unpublished'];
    case 'trigger':
      return [...new Set((w.publishedSpec ?? w.spec).triggers.map((t) => TRIGGER_LABEL[t.kind]))];
    case 'owner':
      return w.createdBy ? [w.createdBy] : [];
    case 'service':
      return w.service ? [w.service] : [];
    case 'tag':
      return w.tags;
  }
}

/** The "Automate your workflows" banner with four blueprint cards. */
function BlueprintBanner() {
  const start = useStartFromBlueprint();
  return (
    <section className="wfl-banner">
      <div className="wfl-banner-text">
        <h2>Automate your workflows</h2>
        <p>Start from a blueprint: remediate alerts, page the right team, report on errors — then adapt every step.</p>
        <Link to="/automation/blueprints" className="wfl-banner-link">
          Browse All Blueprints <ChevronRight size={14} />
        </Link>
      </div>
      <div className="wfl-banner-cards">
        {BLUEPRINTS.slice(0, 4).map((b) => {
          const w = b.build();
          return (
            <button key={b.key} className="wfl-mini" onClick={() => start(b)} disabled={start.pending}>
              <div className="wfl-mini-logos">
                {b.logos.map((l) => (
                  <LogoChip key={l} logo={l} size={24} />
                ))}
              </div>
              <div className="wfl-mini-title">{b.title}</div>
              <div className="wfl-mini-stats">
                {allSteps(w.spec.steps).length} step{allSteps(w.spec.steps).length === 1 ? '' : 's'} ·{TRIGGER_LABEL[b.trigger]} trigger
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}

function FacetGroup({ title, facet, list, selected, onToggle }: { title: string; facet: keyof Facets; list: Workflow[]; selected: Set<string>; onToggle: (facet: keyof Facets, v: string) => void }) {
  const counts = new Map<string, number>();
  for (const w of list) for (const v of values(w, facet)) counts.set(v, (counts.get(v) ?? 0) + 1);
  if (counts.size === 0) return null;
  return (
    <div className="wfl-facet">
      <div className="wfl-facet-title">{title}</div>
      {[...counts]
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
        .map(([v, n]) => (
          <label key={v} className="wfl-facet-row">
            <input type="checkbox" className="checkbox" checked={selected.has(v)} onChange={() => onToggle(facet, v)} />
            <span className="wfl-facet-value">{v}</span>
            <span className="wfl-facet-count">{n}</span>
          </label>
        ))}
    </div>
  );
}

function ColumnsMenu({ shown, onToggle }: { shown: Set<Column>; onToggle: (c: Column) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="wfl-pop" ref={ref}>
      <button className="btn btn-sm" onClick={() => setOpen((o) => !o)}>
        <Columns3 size={14} /> Columns
      </button>
      {open && (
        <div className="au-menu wfl-pop-menu">
          {COLUMNS.map(([c, label]) => (
            <label key={c} className="au-menu-item wfl-pop-row">
              <input type="checkbox" className="checkbox" checked={shown.has(c)} onChange={() => onToggle(c)} /> {label}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

function SortMenu({ sort, onSort }: { sort: SortKey; onSort: (s: SortKey) => void }) {
  const { open, setOpen, ref } = usePopover();
  const labels: Record<SortKey, string> = { name: 'Name', modified: 'Last modified', executed: 'Last execution' };
  return (
    <div className="wfl-pop" ref={ref}>
      <button className="btn btn-sm" onClick={() => setOpen((o) => !o)}>
        <ArrowDownUp size={14} /> Sort by: {labels[sort]}
      </button>
      {open && (
        <div className="au-menu wfl-pop-menu">
          {(Object.keys(labels) as SortKey[]).map((k) => (
            <button
              key={k}
              className={`au-menu-item wfl-pop-row${k === sort ? ' wfl-pop-active' : ''}`}
              onClick={() => {
                onSort(k);
                setOpen(false);
              }}
            >
              {labels[k]}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function TriggerChips({ kinds }: { kinds: WorkflowTriggerKind[] }) {
  if (!kinds.length) return <span className="wf-muted">None</span>;
  return (
    <span className="wfl-triggers">
      {[...new Set(kinds)].map((k) => (
        <span key={k} className="wfl-trigger" title={TRIGGER_LABEL[k]}>
          {TRIGGER_ICON[k]} {TRIGGER_LABEL[k]}
        </span>
      ))}
    </span>
  );
}

/** Workflow Automation → All Workflows. */
export function WorkflowsList() {
  const navigate = useNavigate();
  const { data, isLoading, error } = useWorkflows();
  const profile = useProfile();
  const [q, setQ] = useState('');
  const [mine, setMine] = useState(false);
  const [facets, setFacets] = useState<Facets>(emptyFacets);
  const [sort, setSort] = useState<SortKey>('modified');
  const [shown, setShown] = useState<Set<Column>>(new Set(COLUMNS.map((c) => c[0])));
  const [perPage, setPerPage] = useState(25);
  const [page, setPage] = useState(0);
  const me = profile.data?.name ?? 'Alex Morgan';

  const all = useMemo(() => data ?? [], [data]);
  const scoped = useMemo(() => {
    const t = q.trim().toLowerCase();
    return all.filter((w) => (!mine || w.createdBy === me) && (!t || `${w.name} ${w.description} ${w.tags.join(' ')} ${w.handle}`.toLowerCase().includes(t)));
  }, [all, q, mine, me]);
  const filtered = useMemo(() => {
    const list = scoped.filter((w) => (Object.keys(facets) as (keyof Facets)[]).every((f) => facets[f].size === 0 || values(w, f).some((v) => facets[f].has(v))));
    const key = (w: Workflow) => (sort === 'name' ? w.name.toLowerCase() : sort === 'modified' ? -w.updatedAt : -(w.lastRun?.createdAt ?? 0));
    return [...list].sort((a, b) => {
      const x = key(a);
      const y = key(b);
      return x < y ? -1 : x > y ? 1 : 0;
    });
  }, [scoped, facets, sort]);
  const pages = Math.max(1, Math.ceil(filtered.length / perPage));
  const rows = filtered.slice(page * perPage, (page + 1) * perPage);
  const toggle = (facet: keyof Facets, v: string) => {
    setPage(0);
    setFacets((f) => {
      const next = { ...f, [facet]: new Set(f[facet]) };
      if (next[facet].has(v)) next[facet].delete(v);
      else next[facet].add(v);
      return next;
    });
  };
  const col = (c: Column) => shown.has(c);

  return (
    <div className="wfl-page">
      <BlueprintBanner />
      <div className="wfl-toolbar">
        <div className="wfl-search">
          <Search size={15} />
          <input
            placeholder="Search workflows by name, description, tag or handle"
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setPage(0);
            }}
          />
        </div>
        <Link to="/automation/workflows/new" className="btn btn-primary">
          <Plus size={14} /> New Workflow
        </Link>
      </div>
      <div className="wfl-body">
        <aside className="wfl-facets">
          <label className="wfl-mine">
            <button className={`switch${mine ? ' on' : ''}`} role="switch" aria-checked={mine} aria-label="My workflows" onClick={() => setMine((m) => !m)} />
            My workflows
          </label>
          <FacetGroup title="Team" facet="team" list={scoped} selected={facets.team} onToggle={toggle} />
          <FacetGroup title="Status" facet="status" list={scoped} selected={facets.status} onToggle={toggle} />
          <FacetGroup title="Trigger" facet="trigger" list={scoped} selected={facets.trigger} onToggle={toggle} />
          <FacetGroup title="Owner" facet="owner" list={scoped} selected={facets.owner} onToggle={toggle} />
          <FacetGroup title="Service" facet="service" list={scoped} selected={facets.service} onToggle={toggle} />
          <FacetGroup title="Tags" facet="tag" list={scoped} selected={facets.tag} onToggle={toggle} />
        </aside>
        <section className="wfl-results">
          <div className="wfl-results-head">
            <span className="wfl-count">
              {filtered.length} workflow{filtered.length === 1 ? '' : 's'}
            </span>
            <span className="wfi-spacer" />
            <SortMenu sort={sort} onSort={setSort} />
            <ColumnsMenu shown={shown} onToggle={(c) => setShown((s) => new Set(s.has(c) ? [...s].filter((x) => x !== c) : [...s, c]))} />
          </div>
          {error ? (
            <div className="wfl-empty">{(error as Error).message}</div>
          ) : isLoading ? (
            <div className="wfl-empty">Loading workflows…</div>
          ) : rows.length === 0 ? (
            <div className="wfl-empty">
              <SearchX size={56} strokeWidth={1.2} />
              <div className="wfl-empty-title">No matching results found</div>
              <div className="wf-muted">{all.length ? 'Try other filters or another search.' : 'Create a workflow or start from a blueprint.'}</div>
            </div>
          ) : (
            <table className="wfl-table">
              <thead>
                <tr>
                  <th>Name</th>
                  {col('triggers') && <th>Triggers</th>}
                  {col('owner') && <th>Owner</th>}
                  {col('modified') && <th>Last Modified</th>}
                  {col('tags') && <th>Tags</th>}
                  {col('executed') && <th>Last Execution</th>}
                </tr>
              </thead>
              <tbody>
                {rows.map((w) => (
                  <tr key={w.id} onClick={() => navigate(`/automation/workflows/${w.id}`)}>
                    <td>
                      <div className="wfl-name">
                        <Link to={`/automation/workflows/${w.id}`} onClick={(e) => e.stopPropagation()}>
                          {w.name}
                        </Link>
                        <span className={`wf-chip ${w.published ? 'wf-chip-published' : 'wf-chip-draft'}`}>{w.published ? 'Published' : 'Draft'}</span>
                      </div>
                      {w.description && <div className="wfl-desc">{w.description}</div>}
                    </td>
                    {col('triggers') && (
                      <td>
                        <TriggerChips kinds={(w.publishedSpec ?? w.spec).triggers.map((t) => t.kind)} />
                      </td>
                    )}
                    {col('owner') && <td>{w.createdBy || '—'}</td>}
                    {col('modified') && <td className="wf-muted">{fmtAgo(w.updatedAt)}</td>}
                    {col('tags') && (
                      <td>
                        <div className="wfl-tags">
                          {w.tags.slice(0, 3).map((t) => (
                            <span key={t} className="tag-chip">
                              {t}
                            </span>
                          ))}
                          {w.tags.length > 3 && <span className="wf-muted">+{w.tags.length - 3}</span>}
                        </div>
                      </td>
                    )}
                    {col('executed') && (
                      <td>
                        {w.lastRun ? (
                          <span className="wfl-last">
                            <RunStatusChip status={w.lastRun.status} /> <span className="wf-muted">{fmtAgo(w.lastRun.createdAt)}</span>
                          </span>
                        ) : (
                          <span className="wf-muted">Never run</span>
                        )}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <div className="wfl-pager">
            <span className="wf-muted">Results per page</span>
            <select
              value={perPage}
              onChange={(e) => {
                setPerPage(Number(e.target.value));
                setPage(0);
              }}
            >
              {[25, 50, 100].map((n) => (
                <option key={n}>{n}</option>
              ))}
            </select>
            <span className="wfi-spacer" />
            <span className="wf-muted">
              {filtered.length ? page * perPage + 1 : 0}–{Math.min(filtered.length, (page + 1) * perPage)} of {filtered.length}
            </span>
            <button className="btn btn-sm" disabled={page === 0} onClick={() => setPage((p) => p - 1)}>
              Previous
            </button>
            <button className="btn btn-sm" disabled={page >= pages - 1} onClick={() => setPage((p) => p + 1)}>
              Next
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}

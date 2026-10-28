import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  Activity,
  BellRing,
  BookOpen,
  Box,
  Boxes,
  ChartLine,
  ChevronDown,
  ChevronRight,
  Database,
  DatabaseZap,
  Download,
  GitFork,
  Globe,
  LayoutDashboard,
  LayoutTemplate,
  MessageSquare,
  PanelLeftClose,
  Play,
  Plus,
  Save,
  ScrollText,
  Search,
  Server,
  Trash2,
  TriangleAlert,
  X,
  Bookmark,
  type LucideIcon,
} from 'lucide-react';
import { runSql, type SavedSqlQuery, type SqlResult, type SqlTableInfo, type SqlTemplate } from '../../api/sqlEditor/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { useAccess } from '../settings/queries';
import { SqlCodeEditor, SqlHighlight, type SqlCodeEditorHandle } from './SqlCodeEditor';
import { cellText, snippet, toCsv, untitledName } from './sqlEditorModel';
import { useSavedQueries, useSavedQueryMutations, useSqlSchema, useSqlTemplates } from './sqlEditorQueries';
import './sqlEditor.css';

const FEEDBACK_URL = 'https://github.com/observecraft/observecraft/issues/new?title=SQL%20Editor%20feedback';
const STORAGE_KEY = 'oc.sqlEditor.tabs';
const GETTING_STARTED = 4;

/** Icon and color of a data category (tables and template cards). */
const SOURCES: Record<string, { icon: LucideIcon; color: string; label: string }> = {
  infrastructure: { icon: Server, color: 'var(--primary)', label: 'Infrastructure' },
  containers: { icon: Box, color: 'var(--chart-1)', label: 'Containers' },
  kubernetes: { icon: Boxes, color: 'var(--qv-blue)', label: 'Kubernetes' },
  apm: { icon: Activity, color: 'var(--tile-apm)', label: 'APM' },
  logs: { icon: ScrollText, color: 'var(--qv-green)', label: 'Logs' },
  metrics: { icon: ChartLine, color: 'var(--chart-4)', label: 'Metrics' },
  ci: { icon: GitFork, color: 'var(--qv-yellow)', label: 'CI Visibility' },
  monitors: { icon: BellRing, color: 'var(--qv-red)', label: 'Monitors' },
  incidents: { icon: TriangleAlert, color: 'var(--status-error)', label: 'Incidents' },
  dashboards: { icon: LayoutDashboard, color: 'var(--chart-3)', label: 'Dashboards' },
  synthetics: { icon: Globe, color: 'var(--chart-7)', label: 'Synthetics' },
};
const sourceOf = (key: string) => SOURCES[key] ?? { icon: Database, color: 'var(--qv-gray)', label: key };

type Panel = 'data' | 'queries' | 'docs' | 'templates';
const RAIL: { key: Panel; label: string; icon: LucideIcon }[] = [
  { key: 'data', label: 'Data', icon: Database },
  { key: 'queries', label: 'Queries', icon: Bookmark },
  { key: 'docs', label: 'Docs', icon: BookOpen },
  { key: 'templates', label: 'Templates', icon: LayoutTemplate },
];

interface EditorTab {
  id: string;
  title: string;
  sql: string;
  /** The saved query this tab edits. */
  savedId?: string;
  team?: string;
}

interface TabRun {
  running: boolean;
  result?: SqlResult;
  error?: string;
}

const newTab = (sql = '', title = untitledName(new Date())): EditorTab => ({ id: `t-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, title, sql });

function loadTabs(): EditorTab[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const tabs = raw ? (JSON.parse(raw) as EditorTab[]) : [];
    if (Array.isArray(tabs) && tabs.length > 0 && tabs.every((t) => typeof t.id === 'string' && typeof t.sql === 'string')) return tabs;
  } catch {
    /* storage unavailable or unreadable */
  }
  return [newTab()];
}

/** Dashboards → SQL Editor: read-only SQL over the org's virtual tables, with templates and saved queries. */
export function SqlEditorPage() {
  const [tabs, setTabs] = useState<EditorTab[]>(loadTabs);
  const [activeId, setActiveId] = useState(() => tabs[0].id);
  const [runs, setRuns] = useState<Record<string, TabRun>>({});
  const [panel, setPanel] = useState<Panel | null>('templates');
  const [saving, setSaving] = useState(false);
  const editor = useRef<SqlCodeEditorHandle>(null);
  const { range } = useTime();
  const active = tabs.find((t) => t.id === activeId) ?? tabs[0];

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(tabs));
    } catch {
      /* storage unavailable */
    }
  }, [tabs]);

  const patchTab = useCallback((id: string, patch: Partial<EditorTab>) => setTabs((ts) => ts.map((t) => (t.id === id ? { ...t, ...patch } : t))), []);

  const run = useCallback(
    async (tab: EditorTab) => {
      if (!tab.sql.trim()) return;
      setRuns((r) => ({ ...r, [tab.id]: { ...r[tab.id], running: true, error: undefined } }));
      try {
        const result = await runSql({ sql: tab.sql, range });
        setRuns((r) => ({ ...r, [tab.id]: { running: false, result } }));
      } catch (e) {
        setRuns((r) => ({ ...r, [tab.id]: { running: false, error: e instanceof Error ? e.message : 'The query failed' } }));
      }
    },
    [range],
  );

  /** Opens SQL in the current tab when it is empty, else in a new tab, and runs it. */
  const openSql = (sql: string, title?: string, saved?: SavedSqlQuery) => {
    const fields = { sql, title: title ?? untitledName(new Date()), savedId: saved?.id, team: saved?.team };
    let tab: EditorTab;
    if (!active.sql.trim() && !active.savedId) {
      tab = { ...active, ...fields };
      patchTab(active.id, fields);
    } else {
      tab = { ...newTab(), ...fields };
      setTabs((ts) => [...ts, tab]);
      setActiveId(tab.id);
    }
    run(tab);
  };

  const addTab = () => {
    const tab = newTab();
    setTabs((ts) => [...ts, tab]);
    setActiveId(tab.id);
  };

  const closeTab = (id: string) => {
    setTabs((ts) => {
      const rest = ts.filter((t) => t.id !== id);
      const next = rest.length > 0 ? rest : [newTab()];
      if (id === activeId) setActiveId(next[Math.max(0, ts.findIndex((t) => t.id === id) - 1)]?.id ?? next[0].id);
      return next;
    });
    setRuns((r) => {
      const { [id]: _, ...rest } = r;
      return rest;
    });
  };

  const togglePanel = (p: Panel) => setPanel((cur) => (cur === p ? null : p));
  const current = runs[active.id];

  return (
    <div className="sqle">
      <header className="product-header">
        <div className="product-title">
          <DatabaseZap size={20} /> SQL Editor <span className="badge-new">PREVIEW</span>
        </div>
        <a className="sqle-feedback" href={FEEDBACK_URL} target="_blank" rel="noreferrer">
          <MessageSquare size={14} /> Give feedback
        </a>
      </header>
      <div className="sqle-body">
        <nav className="sqle-rail" aria-label="SQL Editor panels">
          {RAIL.map(({ key, label, icon: Icon }) => (
            <button key={key} className={`sqle-rail-btn${panel === key ? ' active' : ''}`} onClick={() => togglePanel(key)} aria-pressed={panel === key}>
              <Icon size={18} />
              <span>{label}</span>
            </button>
          ))}
        </nav>
        {panel && (
          <aside className="sqle-panel">
            <div className="sqle-panel-head">
              <h2>{panel === 'templates' ? 'Getting Started' : RAIL.find((r) => r.key === panel)?.label}</h2>
              <button className="icon-plain" title="Collapse panel" onClick={() => setPanel(null)}>
                <PanelLeftClose size={16} />
              </button>
            </div>
            <div className="sqle-panel-body">
              {panel === 'templates' && <TemplatesPanel onOpen={(t) => openSql(t.sql, t.title)} />}
              {panel === 'data' && <DataPanel onInsert={(text) => editor.current?.insert(text)} onQuery={(t) => openSql(`SELECT *\nFROM ${t.name}\nLIMIT 100`, t.name)} />}
              {panel === 'queries' && <QueriesPanel activeSavedId={active.savedId} onOpen={(q) => openSql(q.sql, q.name, q)} />}
              {panel === 'docs' && <DocsPanel />}
            </div>
          </aside>
        )}
        <section className="sqle-main">
          <div className="sqle-tabs" role="tablist">
            {tabs.map((t) => (
              <div key={t.id} role="tab" aria-selected={t.id === active.id} className={`sqle-tab${t.id === active.id ? ' active' : ''}`} onClick={() => setActiveId(t.id)}>
                <span className="sqle-tab-title" title={t.title}>
                  {t.title}
                </span>
                <button
                  className="sqle-tab-close"
                  title="Close tab"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(t.id);
                  }}
                >
                  <X size={12} />
                </button>
              </div>
            ))}
            <button className="sqle-tab-add" title="New query tab" onClick={addTab}>
              <Plus size={14} />
            </button>
          </div>
          <div className="sqle-titlebar">
            <input className="sqle-title-input" value={active.title} onChange={(e) => patchTab(active.id, { title: e.target.value })} aria-label="Query name" />
            <div className="sqle-titlebar-actions">
              <TimePicker compact />
              <button className="btn" onClick={() => setSaving(true)} disabled={!active.sql.trim()}>
                <Save size={14} /> {active.savedId ? 'Save' : 'Save query'}
              </button>
              <button className="btn sqle-run" onClick={() => run(active)} disabled={!active.sql.trim() || current?.running} title="Run (Ctrl+Enter)">
                <Play size={14} /> Run
              </button>
            </div>
          </div>
          <SqlCodeEditor ref={editor} value={active.sql} onChange={(sql) => patchTab(active.id, { sql })} onRun={() => run(active)} />
          <Results run={current} title={active.title} />
        </section>
      </div>
      {saving && <SaveDialog tab={active} onClose={() => setSaving(false)} onSaved={(q) => patchTab(active.id, { savedId: q.id, title: q.name, team: q.team })} />}
    </div>
  );
}

// ---------------------------------------------------------------- panels

function TemplateCard({ t, onOpen }: { t: SqlTemplate; onOpen: (t: SqlTemplate) => void }) {
  const src = sourceOf(t.source);
  const Icon = src.icon;
  return (
    <button className="sqle-card" onClick={() => onOpen(t)} title={t.description}>
      <div className="sqle-card-head">
        <span className="sqle-card-icon" style={{ background: src.color }}>
          <Icon size={14} />
        </span>
        <span className="sqle-card-title">{t.title}</span>
      </div>
      <SqlHighlight sql={snippet(t.sql)} className="sqle-card-sql" />
    </button>
  );
}

function TemplatesPanel({ onOpen }: { onOpen: (t: SqlTemplate) => void }) {
  const q = useSqlTemplates();
  const [all, setAll] = useState(false);
  if (q.isPending) return <Loading label="Loading templates…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const shown = all ? q.data : q.data.slice(0, GETTING_STARTED);
  return (
    <>
      <h3 className="sqle-section-title">Query templates</h3>
      <p className="sqle-hint">Start from a query over your hosts, services, logs and monitors. Opening one runs it.</p>
      <div className="sqle-cards">
        {shown.map((t) => (
          <TemplateCard key={t.id} t={t} onOpen={onOpen} />
        ))}
      </div>
      {q.data.length > GETTING_STARTED && (
        <button className="link-btn sqle-more" onClick={() => setAll((a) => !a)}>
          {all ? 'Show fewer templates' : `See more templates (${q.data.length - GETTING_STARTED})`}
        </button>
      )}
    </>
  );
}

function DataPanel({ onInsert, onQuery }: { onInsert: (text: string) => void; onQuery: (t: SqlTableInfo) => void }) {
  const q = useSqlSchema();
  const [filter, setFilter] = useState('');
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const tables = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (q.data?.tables ?? []).filter((t) => !f || t.name.includes(f) || t.columns.some((c) => c.name.includes(f)));
  }, [q.data, filter]);
  if (q.isPending) return <Loading label="Loading tables…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <>
      <label className="sqle-search">
        <Search size={14} />
        <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter tables and columns" aria-label="Filter tables and columns" />
      </label>
      <p className="sqle-hint">Tables hold your organization's data only. Telemetry tables cover the selected time range.</p>
      <ul className="sqle-tables">
        {tables.map((t) => {
          const src = sourceOf(t.category);
          const Icon = src.icon;
          const expanded = open[t.name] ?? false;
          return (
            <li key={t.name}>
              <div className="sqle-table-row">
                <button className="sqle-table-name" onClick={() => setOpen((o) => ({ ...o, [t.name]: !expanded }))} aria-expanded={expanded} title={t.description}>
                  {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <Icon size={14} style={{ color: src.color }} />
                  <span>{t.name}</span>
                </button>
                <button className="link-btn sqle-table-query" onClick={() => onQuery(t)}>
                  Query
                </button>
              </div>
              {expanded && (
                <>
                  <p className="sqle-table-doc">{t.description}</p>
                  <ul className="sqle-columns">
                    {t.columns.map((c) => (
                      <li key={c.name} title={c.description}>
                        <button className="sqle-column" onClick={() => onInsert(c.name)}>
                          {c.name}
                        </button>
                        <span className="sqle-type">{c.type}</span>
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}

function QueriesPanel({ activeSavedId, onOpen }: { activeSavedId?: string; onOpen: (q: SavedSqlQuery) => void }) {
  const q = useSavedQueries();
  const { remove } = useSavedQueryMutations();
  const { canWrite } = useAccess();
  if (q.isPending) return <Loading label="Loading saved queries…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.length === 0) return <p className="sqle-hint">No saved queries yet. Write a query and choose Save query to keep it for your team.</p>;
  return (
    <ul className="sqle-saved">
      {q.data.map((s) => (
        <li key={s.id} className={s.id === activeSavedId ? 'active' : ''}>
          <button className="sqle-saved-open" onClick={() => onOpen(s)}>
            <span className="sqle-saved-name">{s.name}</span>
            <span className="sqle-saved-meta">
              {s.team && <span className="sqle-team">{s.team}</span>}
              {s.updatedBy || s.author} · {timeAgo(s.updatedAt)}
            </span>
          </button>
          {canWrite && (
            <button
              className="icon-plain"
              title="Delete query"
              onClick={() => {
                if (window.confirm(`Delete “${s.name}”?`)) remove.mutate(s.id);
              }}
            >
              <Trash2 size={14} />
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

function DocsPanel() {
  const q = useSqlSchema();
  const [filter, setFilter] = useState('');
  const functions = (q.data?.functions ?? []).filter((f) => f.toLowerCase().includes(filter.trim().toLowerCase()));
  return (
    <div className="sqle-docs">
      <DocSection title="Queries">
        <p>
          Write one <code>SELECT</code>. <code>WITH</code>, <code>JOIN</code>, <code>UNION ALL</code>, <code>GROUP BY</code>, <code>ORDER BY</code>, <code>LIMIT</code>, subqueries and window functions
          (<code>OVER (PARTITION BY …)</code>) are supported. The dialect is ClickHouse SQL: <code>count()</code>, <code>countIf(x)</code>, <code>quantile(0.95)(x)</code>, <code>{"tags['env']"}</code>{' '}
          for map values, <code>{'arrayMap(x -> x * 2, arr)'}</code> for arrays.
        </p>
        <SqlHighlight sql={"SELECT service, quantile(0.95)(duration_ms) AS p95\nFROM spans\nWHERE kind = 'Server'\nGROUP BY service\nORDER BY p95 DESC"} />
      </DocSection>
      <DocSection title="Tables">
        <p>
          Queries read the tables listed under <b>Data</b>, which hold only your organization's data. <code>spans</code>, <code>services</code>, <code>logs</code>, <code>metrics</code>,{' '}
          <code>ci_pipelines</code> and <code>hosts</code> cover the time range picked next to Run (at most 7 days); <code>logs</code> follows your log access restrictions. The other tables show
          current state.
        </p>
      </DocSection>
      <DocSection title="Limits">
        <p>
          Results stop at {(q.data?.maxRows ?? 10000).toLocaleString()} rows; a query runs for at most 30 seconds. Statements that change data, settings, table functions and server tables are refused.
        </p>
      </DocSection>
      <DocSection title={`Functions (${q.data?.functions.length ?? 0})`}>
        <label className="sqle-search">
          <Search size={14} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter functions" aria-label="Filter functions" />
        </label>
        <div className="sqle-fns">
          {functions.map((f) => (
            <code key={f}>{f}</code>
          ))}
        </div>
      </DocSection>
    </div>
  );
}

function DocSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="sqle-doc">
      <h3 className="sqle-section-title">{title}</h3>
      {children}
    </section>
  );
}

// ---------------------------------------------------------------- results

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = `${name.replace(/[^\w.-]+/g, '_') || 'query'}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}

function Results({ run, title }: { run?: TabRun; title: string }) {
  if (!run) return <div className="sqle-results sqle-results-empty">Run a query to see results here. Press Ctrl+Enter to run.</div>;
  if (run.running && !run.result) return <div className="sqle-results">{<Loading label="Running query…" />}</div>;
  return (
    <div className="sqle-results">
      {run.error && (
        <div className="sqle-error" role="alert">
          <TriangleAlert size={14} /> {run.error}
        </div>
      )}
      {run.result && (
        <>
          <div className="sqle-results-bar">
            <span>
              <b>{run.result.rows.length.toLocaleString()}</b> {run.result.rows.length === 1 ? 'row' : 'rows'}
              {run.result.truncated && <span className="sqle-truncated"> (first {run.result.rows.length.toLocaleString()} shown)</span>}
            </span>
            <span className="sqle-muted">{run.result.elapsedMs.toLocaleString()} ms</span>
            {run.running && <span className="sqle-muted">Running…</span>}
            {run.result.notices.map((n) => (
              <span key={n} className="sqle-notice">
                {n}
              </span>
            ))}
            <button
              className="btn btn-sm sqle-csv"
              disabled={run.result.columns.length === 0}
              onClick={() =>
                download(
                  title,
                  toCsv(
                    run.result!.columns.map((c) => c.name),
                    run.result!.rows,
                  ),
                )
              }
            >
              <Download size={13} /> CSV
            </button>
          </div>
          <div className="sqle-grid-wrap">
            <table className="sqle-grid">
              <thead>
                <tr>
                  <th className="sqle-rownum">#</th>
                  {run.result.columns.map((c, i) => (
                    <th key={i} title={c.type}>
                      {c.name}
                      <span className="sqle-type">{c.type}</span>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {run.result.rows.map((row, r) => (
                  <tr key={r}>
                    <td className="sqle-rownum">{r + 1}</td>
                    {row.map((v, c) => (
                      <td key={c} className={v === null ? 'sqle-null' : typeof v === 'number' ? 'sqle-num' : ''}>
                        {cellText(v)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
            {run.result.rows.length === 0 && <div className="sqle-results-empty">No rows.</div>}
          </div>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- save

function SaveDialog({ tab, onClose, onSaved }: { tab: EditorTab; onClose: () => void; onSaved: (q: SavedSqlQuery) => void }) {
  const { create, update } = useSavedQueryMutations();
  const [name, setName] = useState(tab.title.startsWith('Untitled') ? '' : tab.title);
  const [team, setTeam] = useState(tab.team ?? '');
  const [asNew, setAsNew] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = create.isPending || update.isPending;
  const save = async () => {
    setError(null);
    try {
      const input = { name, sql: tab.sql, team };
      const q = tab.savedId && !asNew ? await update.mutateAsync({ id: tab.savedId, input }) : await create.mutateAsync(input);
      onSaved(q);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Saving failed');
    }
  };
  return (
    <Modal onClose={onClose} label="Save query" width={460} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <h3>{tab.savedId ? 'Save query' : 'Save new query'}</h3>
        <label className="sqle-field">
          Name
          <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Hosts by operating system" maxLength={200} />
        </label>
        <label className="sqle-field">
          Team (optional)
          <input value={team} onChange={(e) => setTeam(e.target.value)} placeholder="sre" maxLength={200} />
        </label>
        {tab.savedId && (
          <label className="sqle-check">
            <input type="checkbox" checked={asNew} onChange={(e) => setAsNew(e.target.checked)} /> Save as a new query
          </label>
        )}
        <p className="sqle-muted">Saved queries are visible to everyone in your organization.</p>
        {error && <p className="sqle-error">{error}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={pending || !name.trim()}>
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}

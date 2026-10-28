import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { AlertTriangle, ArrowDown, ArrowLeft, ArrowUp, ExternalLink, LayoutGrid, Save, Trash2 } from 'lucide-react';
import type { AppComponent, AppComponentKind, AppSpec, AppValues } from '../../../api/apps/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { useWorkflows } from '../queries';
import { initialValues, KIND_LABEL, moveComponent, newComponent, PALETTE, renameComponent, specProblem } from './appLogic';
import { AppRenderer } from './AppRenderer';
import { BuilderProperties } from './BuilderProperties';
import { KIND_ICON } from './kindIcons';
import { useApp, useSaveApp } from './queries';
import './apps.css';

interface Draft {
  name: string;
  description: string;
  tags: string[];
  blueprint: string;
  published: boolean;
  spec: AppSpec;
}

const EMPTY: Draft = { name: 'Untitled app', description: '', tags: [], blueprint: '', published: false, spec: { components: [] } };

/** App Builder: component palette and list, a live preview on the grid, and the selected component's properties. */
export function AppBuilderPage() {
  const { id } = useParams();
  const app = useApp(id);
  if (id && app.isLoading) return <Loading />;
  if (id && (app.isError || !app.data)) return <ErrorState error={app.error} onRetry={() => app.refetch()} />;
  const initial: Draft = app.data
    ? { name: app.data.name, description: app.data.description, tags: app.data.tags, blueprint: app.data.blueprint, published: app.data.published, spec: app.data.spec }
    : EMPTY;
  return <Builder key={id ?? 'new'} id={id} initial={initial} />;
}

function Builder({ id, initial }: { id?: string; initial: Draft }) {
  const navigate = useNavigate();
  const save = useSaveApp();
  const workflows = useWorkflows();
  const [draft, setDraft] = useState<Draft>(initial);
  const [saved, setSaved] = useState<Draft>(initial);
  const [selected, setSelected] = useState<string | null>(initial.spec.components[0]?.id ?? null);
  const [values, setValues] = useState<AppValues>(() => initialValues(initial.spec));
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const problem = specProblem(draft.spec);
  const current = draft.spec.components.find((c) => c.id === selected) ?? null;

  // Keep preview values in line with defaults when components change.
  const defaults = useMemo(() => initialValues(draft.spec), [draft.spec]);
  useEffect(() => {
    setValues((v) => ({ ...defaults, ...Object.fromEntries(Object.entries(v).filter(([k]) => k in defaults)) }));
  }, [defaults]);

  const setSpec = (spec: AppSpec) => setDraft((d) => ({ ...d, spec }));
  const add = (kind: AppComponentKind) => {
    const c = newComponent(draft.spec, kind);
    const index = current ? draft.spec.components.findIndex((x) => x.id === current.id) + 1 : draft.spec.components.length;
    setSpec({ components: [...draft.spec.components.slice(0, index), c, ...draft.spec.components.slice(index)] });
    setSelected(c.id);
  };
  const change = (next: AppComponent) => setSpec({ components: draft.spec.components.map((c) => (c.id === next.id ? next : c)) });
  const remove = (cid: string) => {
    setSpec({ components: draft.spec.components.filter((c) => c.id !== cid) });
    if (selected === cid) setSelected(null);
  };

  const persist = (next: Draft, then?: (id: string) => void) => {
    setError(null);
    if (!next.name.trim()) return setError('The name is required');
    save.mutate(
      { id, input: { name: next.name, description: next.description, tags: next.tags, blueprint: next.blueprint, published: next.published, spec: next.spec } },
      {
        onSuccess: (a) => {
          setSaved(next);
          setDraft(next);
          setNotice(next.published ? 'Saved and published' : 'Saved');
          setTimeout(() => setNotice(null), 2500);
          if (!id) navigate(`/automation/apps/${a.id}/edit`, { replace: true });
          then?.(a.id);
        },
        onError: (e) => setError(e instanceof Error ? e.message : 'The app could not be saved'),
      },
    );
  };

  return (
    <div className="ab-builder">
      <header className="ab-topbar">
        <Link to={id ? `/automation/apps/${id}` : '/automation/apps'} className="ab-icon-btn" aria-label="Back">
          <ArrowLeft size={16} />
        </Link>
        <span className="ab-app-icon">
          <LayoutGrid size={16} />
        </span>
        <input className="ab-name" value={draft.name} aria-label="App name" onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
        <span className={`ab-chip ${saved.published ? 'ab-chip-published' : 'ab-chip-draft'}`}>{saved.published ? 'Published' : 'Draft'}</span>
        {dirty && <span className="ab-unsaved">Unsaved changes</span>}
        <div className="ab-topbar-right">
          {notice && <span className="ab-notice">{notice}</span>}
          {problem && (
            <span className="ab-topbar-problem" title={problem}>
              <AlertTriangle size={13} /> {problem}
            </span>
          )}
          {id && (
            <Link className="btn" to={`/automation/apps/${id}`}>
              <ExternalLink size={13} /> Open app
            </Link>
          )}
          <button className="btn" disabled={save.isPending || !!problem} onClick={() => persist(draft)}>
            <Save size={13} /> Save
          </button>
          <button className="btn btn-primary ab-publish" disabled={save.isPending || !!problem} onClick={() => persist({ ...draft, published: !saved.published })}>
            {saved.published ? 'Unpublish' : 'Publish'}
          </button>
        </div>
      </header>
      {error && (
        <div className="ab-alert" role="alert">
          {error}
        </div>
      )}
      <div className="ab-body">
        <aside className="ab-left">
          <div className="ab-panel-title">Components</div>
          {PALETTE.map((g) => (
            <div key={g.group} className="ab-palette-group">
              <div className="ab-palette-label">{g.group}</div>
              <div className="ab-palette">
                {g.kinds.map((k) => {
                  const Icon = KIND_ICON[k];
                  return (
                    <button key={k} className="ab-palette-item" onClick={() => add(k)} title={`Add a ${KIND_LABEL[k].toLowerCase()}`}>
                      <Icon size={16} />
                      <span>{KIND_LABEL[k]}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
          <div className="ab-panel-title ab-panel-title-gap">In this app ({draft.spec.components.length})</div>
          <ul className="ab-outline">
            {draft.spec.components.map((c, i) => {
              const Icon = KIND_ICON[c.kind];
              return (
                <li key={c.id} className={selected === c.id ? 'active' : ''}>
                  <button className="ab-outline-main" onClick={() => setSelected(c.id)}>
                    <Icon size={13} />
                    <span className="mono">{c.id}</span>
                    <span className="ab-outline-kind">{KIND_LABEL[c.kind]}</span>
                  </button>
                  <span className="ab-outline-actions">
                    <button aria-label={`Move ${c.id} up`} disabled={i === 0} onClick={() => setSpec(moveComponent(draft.spec, c.id, -1))}>
                      <ArrowUp size={12} />
                    </button>
                    <button aria-label={`Move ${c.id} down`} disabled={i === draft.spec.components.length - 1} onClick={() => setSpec(moveComponent(draft.spec, c.id, 1))}>
                      <ArrowDown size={12} />
                    </button>
                    <button aria-label={`Delete ${c.id}`} onClick={() => remove(c.id)}>
                      <Trash2 size={12} />
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
        </aside>
        <main className="ab-canvas">
          <div className="ab-frame">
            <div className="ab-frame-bar">
              <span className="ab-dot" />
              <span className="ab-dot" />
              <span className="ab-dot" />
              <span className="ab-frame-title">{draft.name || 'Untitled app'}</span>
              <span className="ab-frame-hint">Preview — buttons don’t run here</span>
            </div>
            <div className="ab-app-surface">
              <AppRenderer
                spec={draft.spec}
                values={values}
                onValue={(k, v) => setValues((p) => ({ ...p, [k]: v }))}
                onPress={(b) => setSelected(b.id)}
                runs={{}}
                selected={selected}
                onSelect={setSelected}
              />
            </div>
          </div>
        </main>
        <aside className="ab-right">
          <div className="ab-panel-title">{current ? 'Properties' : 'App'}</div>
          {current ? (
            <BuilderProperties
              key={current.id}
              c={current}
              spec={draft.spec}
              workflows={workflows.data ?? []}
              onChange={change}
              onRename={(to) => {
                setSpec(renameComponent(draft.spec, current.id, to));
                setSelected(to);
              }}
            />
          ) : (
            <div className="ab-props">
              <div className="ab-prop">
                <span className="ab-prop-label">Description</span>
                <textarea className="ab-input ab-textarea" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
              </div>
              <div className="ab-prop">
                <span className="ab-prop-label">Tags</span>
                <TagsInput tags={draft.tags} onChange={(tags) => setDraft({ ...draft, tags })} />
              </div>
              <p className="ab-prop-hint">Select a component on the canvas to edit it, or add one from the left panel.</p>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}

function TagsInput({ tags, onChange }: { tags: string[]; onChange: (tags: string[]) => void }) {
  const [text, setText] = useState(tags.join(', '));
  return (
    <input
      className="ab-input"
      value={text}
      placeholder="team:checkout, ops"
      onChange={(e) => setText(e.target.value)}
      onBlur={() =>
        onChange(
          text
            .split(',')
            .map((t) => t.trim())
            .filter(Boolean),
        )
      }
    />
  );
}

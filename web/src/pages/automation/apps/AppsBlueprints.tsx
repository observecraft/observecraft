import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Info, Plus } from 'lucide-react';
import type { AppComponentKind } from '../../../api/apps/types';
import type { Workflow } from '../../../api/workflows/types';
import { useWorkflows } from '../queries';
import { APP_BLUEPRINTS, type AppBlueprint } from './blueprints';
import { KIND_ICON } from './kindIcons';
import { useApps, useSaveApp } from './queries';
import './apps.css';

const runnable = (w: Workflow) => !!w.publishedSpec && w.publishedSpec.triggers.some((t) => t.kind === 'app');

/** A free app name: `Service operations`, `Service operations (2)`, … */
function freeName(base: string, taken: string[]): string {
  if (!taken.includes(base)) return base;
  for (let i = 2; ; i++) if (!taken.includes(`${base} (${i})`)) return `${base} (${i})`;
}

/** App blueprints: ready-made apps that open in the builder. */
export function AppsBlueprints() {
  const navigate = useNavigate();
  const workflows = useWorkflows();
  const apps = useApps();
  const save = useSaveApp();
  const [error, setError] = useState<string | null>(null);
  const workflow = (workflows.data ?? []).find(runnable);

  const use = (b: AppBlueprint) => {
    setError(null);
    const taken = (apps.data ?? []).map((a) => a.name);
    save.mutate(
      { input: { name: freeName(b.title, taken), description: b.description, tags: b.tags, blueprint: b.id, published: false, spec: b.spec(b.needsWorkflow ? (workflow?.id ?? '') : '') } },
      {
        onSuccess: (a) => navigate(`/automation/apps/${a.id}/edit`),
        onError: (e) => setError(e instanceof Error ? e.message : 'The app could not be created'),
      },
    );
  };

  return (
    <div className="ab-list-page">
      <div className="ab-bp-intro">
        <Info size={15} />
        <span>
          Blueprints open in the builder as drafts. Buttons run a workflow published with an <b>App</b> trigger
          {workflow ? (
            <>
              ; they start out wired to <b>{workflow.name}</b>.
            </>
          ) : (
            <>; none is published yet, so pick one in the button’s properties.</>
          )}
        </span>
      </div>
      {error && <div className="ab-alert">{error}</div>}
      <div className="ab-bp-grid">
        <button className="ab-bp-card ab-bp-blank" onClick={() => navigate('/automation/apps/new')}>
          <Plus size={22} />
          <strong>Start from scratch</strong>
          <span>An empty canvas</span>
        </button>
        {APP_BLUEPRINTS.map((b) => {
          const kinds = [...new Set(b.spec('').components.map((c) => c.kind))] as AppComponentKind[];
          return (
            <div key={b.id} className="ab-bp-card">
              <div className="ab-bp-thumb">
                {b
                  .spec('')
                  .components.slice(0, 7)
                  .map((c) => (
                    <span key={c.id} className={`ab-bp-block ab-bp-${c.kind}`} style={{ gridColumn: `span ${c.width}` }} />
                  ))}
              </div>
              <h3>{b.title}</h3>
              <p>{b.description}</p>
              <div className="ab-bp-kinds">
                {kinds.map((k) => {
                  const Icon = KIND_ICON[k];
                  return (
                    <span key={k} className="ab-bp-kind" title={k}>
                      <Icon size={12} />
                    </span>
                  );
                })}
                {b.needsWorkflow && !workflow && <span className="ab-bp-note">Needs a workflow</span>}
              </div>
              <button className="btn ab-bp-use" disabled={save.isPending} onClick={() => use(b)}>
                Use blueprint
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}

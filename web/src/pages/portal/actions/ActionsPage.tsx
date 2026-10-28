import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Info, Play, Plus, Search, X } from 'lucide-react';
import { BLUEPRINTS, blueprint as findBlueprint, type Blueprint } from '../../../api/portalActions/blueprints';
import { CATEGORY_LABEL } from '../../../api/portalActions/logic';
import type { PortalActionCategory } from '../../../api/portalActions/client';
import { Modal } from '../../../components/ui/Modal';
import { ActionForm, actionLogo, initialValues, RunStatus, ToolLogo } from './common';
import { usePortalActions } from './queries';
import './actions.css';

const BANNER_KEY = 'oc.portal.actions.banner-dismissed';
const PILLS: (PortalActionCategory | 'all')[] = ['all', 'scaffolding', 'release_management', 'infrastructure_provisioning', 'incident_management', 'infrastructure_management'];

function bannerDismissed(): boolean {
  try {
    return localStorage.getItem(BANNER_KEY) === '1';
  } catch {
    return false;
  }
}

/** Self-Service Actions: the blueprint gallery and the org's saved actions. */
export function ActionsPage() {
  const [banner, setBanner] = useState(() => !bannerDismissed());
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<PortalActionCategory | 'all'>('all');
  const [preview, setPreview] = useState<Blueprint | null>(null);
  const actions = usePortalActions();

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return BLUEPRINTS.filter((b) => (category === 'all' || b.category === category) && (!q || `${b.title} ${b.description}`.toLowerCase().includes(q)));
  }, [query, category]);

  const dismiss = () => {
    setBanner(false);
    try {
      localStorage.setItem(BANNER_KEY, '1');
    } catch {
      /* storage unavailable: the banner just comes back next time */
    }
  };

  return (
    <div className="psa-page">
      {banner && (
        <div className="psa-banner" role="status">
          <Info size={16} />
          <span>
            Self-service actions turn routine requests — new repositories, deployments, infrastructure changes, incidents — into forms any developer can run. Credentials stay on the server and every
            run is recorded.
          </span>
          <button className="sc-icon-btn" aria-label="Dismiss" onClick={dismiss}>
            <X size={16} />
          </button>
        </div>
      )}

      {actions.data && actions.data.length > 0 && (
        <section className="psa-section">
          <div className="psa-section-head">
            <h2>Your actions</h2>
            <Link className="btn btn-primary" to="/portal/actions/new">
              <Plus size={14} /> New action
            </Link>
          </div>
          <table className="psa-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Category</th>
                <th>Owner</th>
                <th>Runs</th>
                <th>Last run</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {actions.data.map((a) => (
                <tr key={a.id}>
                  <td>
                    <Link className="psa-action-name" to={`/portal/actions/${a.id}`}>
                      <ToolLogo logo={actionLogo(findBlueprint(a.blueprint)?.logo, a.step.kind, a.step.requests?.[0]?.url)} size={16} />
                      {a.name}
                    </Link>
                    {a.description && <div className="psa-muted psa-clip">{a.description}</div>}
                  </td>
                  <td>{CATEGORY_LABEL[a.category]}</td>
                  <td>{a.ownerTeam ? `@${a.ownerTeam}` : <span className="psa-muted">—</span>}</td>
                  <td>{a.runs}</td>
                  <td>
                    <RunStatus status={a.lastRunStatus} />
                    {a.lastRunAt && <span className="psa-muted"> · {new Date(a.lastRunAt).toLocaleString()}</span>}
                  </td>
                  <td className="psa-right">
                    <Link className="btn btn-sm" to={`/portal/actions/${a.id}`}>
                      <Play size={13} /> Run
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
      {actions.isError && <div className="form-error">{(actions.error as Error).message}</div>}

      <section className="psa-section">
        <h1 className="psa-title">Get Started With Blueprints</h1>
        <p className="psa-lede">
          Blueprints are ready-made actions for the requests platform teams answer every week. Preview one, adjust its form and credentials, and publish it — or{' '}
          <Link to="/portal/actions/new">build your own</Link> from scratch.
        </p>
        <div className="psa-search">
          <Search size={15} />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search blueprints…" aria-label="Search blueprints" />
        </div>
        <div className="psa-pills" role="tablist">
          {PILLS.map((p) => (
            <button key={p} role="tab" aria-selected={category === p} className={`psa-pill${category === p ? ' active' : ''}`} onClick={() => setCategory(p)}>
              {p === 'all' ? 'All' : CATEGORY_LABEL[p]}
            </button>
          ))}
        </div>
        {shown.length === 0 ? (
          <p className="psa-muted psa-empty">No blueprint matches “{query}”.</p>
        ) : (
          <div className="psa-grid">
            {shown.map((b) => (
              <BlueprintCard key={b.key} b={b} onPreview={() => setPreview(b)} />
            ))}
          </div>
        )}
      </section>
      {preview && <PreviewModal b={preview} onClose={() => setPreview(null)} />}
    </div>
  );
}

function BlueprintCard({ b, onPreview }: { b: Blueprint; onPreview: () => void }) {
  return (
    <article className="psa-card">
      <div className="psa-card-copy">
        <h3>{b.title}</h3>
        <p>{b.description}</p>
        <button className="btn psa-preview-btn" onClick={onPreview}>
          Preview App
        </button>
      </div>
      <div className="psa-card-art" aria-hidden="true">
        {b.suggested && <span className="psa-ribbon">Suggested</span>}
        <MiniForm b={b} />
        <span className="psa-card-logo">
          <ToolLogo logo={b.logo} size={22} />
        </span>
      </div>
    </article>
  );
}

/** A light sketch of the blueprint's form: its first fields and a submit button. */
function MiniForm({ b }: { b: Blueprint }) {
  const fields = b.inputs.slice(0, 3);
  return (
    <div className="psa-mini">
      <div className="psa-mini-title">{b.title}</div>
      {fields.map((f) => (
        <div key={f.key} className="psa-mini-field">
          <span className="psa-mini-label">{f.label}</span>
          <span className={`psa-mini-input${f.kind === 'select' ? ' select' : ''}`}>{f.default || ''}</span>
        </div>
      ))}
      <span className="psa-mini-button">Run</span>
    </div>
  );
}

function PreviewModal({ b, onClose }: { b: Blueprint; onClose: () => void }) {
  const navigate = useNavigate();
  return (
    <Modal label={`Preview of ${b.title}`} onClose={onClose} width={640} height="auto">
      <div className="sc-modal">
        <header className="sc-modal-head">
          <h2 className="psa-preview-title">
            <ToolLogo logo={b.logo} size={20} /> {b.title}
          </h2>
          <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        <p className="psa-muted">{b.description}</p>
        <div className="psa-preview-frame">
          <ActionForm inputs={b.inputs} values={initialValues(b.inputs)} readOnly />
          <button className="btn btn-primary" disabled>
            <Play size={14} /> Run
          </button>
        </div>
        {b.secrets.length > 0 && (
          <p className="psa-muted">
            Needs {b.secrets.map((s) => s.name).join(', ')}: {b.secrets.map((s) => s.hint).join('; ')}.
          </p>
        )}
        <div className="sc-modal-actions">
          <button className="btn" onClick={onClose}>
            Close
          </button>
          <button className="btn btn-primary" onClick={() => navigate(`/portal/actions/new?blueprint=${b.key}`)}>
            Use blueprint
          </button>
        </div>
      </div>
    </Modal>
  );
}

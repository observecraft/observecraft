import { useState } from 'react';
import { Link, NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { ChevronRight, Monitor, Plus, Search, X } from 'lucide-react';
import type { RumApplication } from '../../api/rum/client';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { APP_KINDS, AppTypeIcon, TYPE_LABEL, type AppKind } from './common';
import { useRumApplications, useSaveRumApplication } from './queries';
import '../monitors/monitors.css';
import './rum.css';

/** RUM → Application Management's left navigation. */
export function RumSettingsNav() {
  return (
    <nav className="rum-settings-nav">
      <NavLink to="/rum/applications" className={({ isActive }) => (isActive ? 'active' : '')}>
        Manage Applications
      </NavLink>
      <NavLink to="/rum/generate-metrics" className={({ isActive }) => (isActive ? 'active' : '')}>
        Generate metrics
      </NavLink>
      <span className="disabled" title="Usage attribution isn't available yet">
        Enforce Usage Attribution
      </span>
    </nav>
  );
}

export function RumCrumbs({ children }: { children?: React.ReactNode }) {
  return (
    <header className="mon-crumbs rum-crumbs">
      <Link to="/rum" className="mon-crumb-root">
        <Monitor size={20} /> RUM
      </Link>
      <ChevronRight size={18} className="muted" />
      <Link to="/rum/applications" className="mon-crumb-link">
        Application Management
      </Link>
      {children}
    </header>
  );
}

/** RUM → Application Management: the org's applications and New Application. */
export function RumApplicationsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const apps = useRumApplications();
  const [search, setSearch] = useState('');
  const creating = params.get('new') === '1';
  const setCreating = (on: boolean) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (on) n.set('new', '1');
        else n.delete('new');
        return n;
      },
      { replace: true },
    );
  const shown = (apps.data ?? []).filter((a) => `${a.name} ${a.type} ${a.id}`.toLowerCase().includes(search.trim().toLowerCase()));
  return (
    <div className="mon-status-page">
      <RumCrumbs />
      <div className="rum-settings">
        <RumSettingsNav />
        <div className="rum-settings-main">
          <div className="rum-apps-top">
            <label className="rum-search">
              <Search size={15} />
              <input placeholder="Search applications by name, type or ID" value={search} onChange={(e) => setSearch(e.target.value)} />
            </label>
            <button className="btn btn-primary" onClick={() => setCreating(true)}>
              <Plus size={15} /> Set Up Manually
            </button>
          </div>
          {apps.isPending ? (
            <Loading label="Loading applications…" />
          ) : apps.isError ? (
            <ErrorState error={apps.error} onRetry={() => apps.refetch()} />
          ) : shown.length === 0 ? (
            <div className="syn-empty">
              <Monitor size={36} strokeWidth={1.4} />
              <h3>{apps.data.length ? 'No application matches' : 'Monitor your first application'}</h3>
              <p className="muted">Create an application, then add the RUM SDK with its application ID and client token to your web or mobile app.</p>
              {!apps.data.length && (
                <button className="btn btn-primary" onClick={() => setCreating(true)}>
                  <Plus size={15} /> New Application
                </button>
              )}
            </div>
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table syn-table rum-apps-table">
                <thead>
                  <tr>
                    <th>NAME</th>
                    <th>TYPE</th>
                    <th>APPLICATION ID</th>
                    <th>PRODUCTS</th>
                    <th>DATE CREATED</th>
                    <th>CREATED BY</th>
                  </tr>
                </thead>
                <tbody>
                  {shown.map((a) => (
                    <tr key={a.id} onClick={() => navigate(`/rum/applications/${a.id}`)}>
                      <td className="rum-app-name">
                        <AppTypeIcon type={a.type} size={22} /> {a.name}
                      </td>
                      <td>{TYPE_LABEL[a.type] ?? a.type}</td>
                      <td className="mono muted">{a.id}</td>
                      <td>
                        <span className={`rum-product${a.rumEventProcessingState === 'NONE' ? ' off' : ''}`}>RUM</span>
                        {a.productAnalyticsRetentionState === 'MAX' && <span className="rum-product">PRODUCT ANALYTICS</span>}
                      </td>
                      <td className="muted">{new Date(a.createdAt).toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' })}</td>
                      <td>{a.createdBy ?? '—'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
      {creating && <NewApplicationModal onClose={() => setCreating(false)} onCreated={(a) => navigate(`/rum/applications/${a.id}?tab=sdk`)} />}
    </div>
  );
}

function KindLogo({ kind }: { kind: AppKind }) {
  if (kind.logo && hasBrandLogo(kind.logo)) {
    return (
      <span className="rum-kind-logo">
        <BrandLogo name={kind.logo} size={34} title={kind.label} />
        <span>{kind.label}</span>
      </span>
    );
  }
  return <span className="rum-kind-word">{kind.label}</span>;
}

/** New Application: a name and a type (the SDK the application uses). */
function NewApplicationModal({ onClose, onCreated }: { onClose: () => void; onCreated: (a: RumApplication) => void }) {
  const [name, setName] = useState('');
  const [kind, setKind] = useState<AppKind>(APP_KINDS[0]);
  const save = useSaveRumApplication();
  const create = () => save.mutate({ name: name.trim(), type: kind.type, tags: kind.type === 'browser' && kind.id !== 'js' ? [`framework:${kind.id}`] : [] }, { onSuccess: onCreated });
  return (
    <Modal onClose={onClose} label="New Application" width={1340} height="auto">
      <div className="rum-newapp">
        <header className="rum-newapp-head">
          <span>MANUAL SETUP</span>
          <button className="icon-plain" title="Close" onClick={onClose}>
            <X size={20} />
          </button>
        </header>
        <div className="rum-newapp-body">
          <h2>New Application</h2>
          <label className="rum-field">
            <span>
              Name <em>*</em>
            </span>
            <input className="syn-input" placeholder="Application Name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <div className="rum-field">
            <span>
              Type <em>*</em>
            </span>
            <div className="rum-kinds" role="radiogroup" aria-label="Application type">
              {APP_KINDS.map((k) => (
                <button
                  key={k.id}
                  role="radio"
                  aria-checked={kind.id === k.id}
                  className={`rum-kind${kind.id === k.id ? ' active' : ''}`}
                  disabled={k.soon}
                  onClick={() => setKind(k)}
                  title={k.soon ? 'Not available yet' : k.label}
                >
                  {k.badge && <em className={`rum-kind-badge ${k.badge.toLowerCase()}`}>{k.badge}</em>}
                  <KindLogo kind={k} />
                </button>
              ))}
            </div>
          </div>
          {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        </div>
        <footer className="rum-newapp-foot">
          <button className="link-btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending} onClick={create}>
            {save.isPending ? 'Creating…' : 'Create Application'}
          </button>
        </footer>
      </div>
    </Modal>
  );
}

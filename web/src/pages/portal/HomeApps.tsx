import { useState } from 'react';
import { AppWindow, ExternalLink, LayoutGrid, Pencil, Plus, Trash2 } from 'lucide-react';
import type { PortalApp } from '../../api/portal/client';
import { guessLogo } from '../../api/portal/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { HomeSection } from './homeCommon';
import { useApps, useCreateApp, useDeleteApp, useUpdateApp } from './homeQueries';

/** Logos offered for apps (keys of the logo registry). */
export const APP_LOGOS: { key: string; name: string }[] = [
  { key: 'github', name: 'GitHub' },
  { key: 'gitlab', name: 'GitLab' },
  { key: 'bitbucket', name: 'Bitbucket' },
  { key: 'jira', name: 'Jira' },
  { key: 'confluence', name: 'Confluence' },
  { key: 'linear', name: 'Linear' },
  { key: 'notion', name: 'Notion' },
  { key: 'slack', name: 'Slack' },
  { key: 'figma', name: 'Figma' },
  { key: 'pagerduty', name: 'PagerDuty' },
  { key: 'sentry', name: 'Sentry' },
  { key: 'terraform', name: 'Terraform' },
  { key: 'launchdarkly', name: 'LaunchDarkly' },
  { key: 'sonarqube', name: 'SonarQube' },
  { key: 'jenkins', name: 'Jenkins' },
  { key: 'circleci', name: 'CircleCI' },
  { key: 'githubactions', name: 'GitHub Actions' },
  { key: 'kubernetes', name: 'Kubernetes' },
  { key: 'aws', name: 'AWS' },
  { key: 'googledrive', name: 'Google Drive' },
  { key: 'postman', name: 'Postman' },
  { key: 'miro', name: 'Miro' },
  { key: 'asana', name: 'Asana' },
  { key: 'trello', name: 'Trello' },
  { key: 'backstage', name: 'Backstage' },
  { key: 'tile:servicenow', name: 'ServiceNow' },
  { key: 'tile:azure-devops', name: 'Azure DevOps' },
];

/** The tilted wall of tool tiles behind the empty state. */
function Mosaic() {
  const tiles = Array.from({ length: 48 }, (_, i) => APP_LOGOS[(i * 7) % APP_LOGOS.length]);
  return (
    <div className="ph-mosaic" aria-hidden="true">
      <div className="ph-mosaic-grid">
        {tiles.map((t, i) => (
          <span key={i} className="ph-mosaic-tile">
            <IntegrationLogo logo={t.key} name={t.name} size={52} />
          </span>
        ))}
      </div>
    </div>
  );
}

function AppModal({ app, onClose }: { app: PortalApp | null; onClose: () => void }) {
  const [name, setName] = useState(app?.name ?? '');
  const [url, setUrl] = useState(app?.url ?? '');
  const [logo, setLogo] = useState(app?.logo ?? '');
  const create = useCreateApp();
  const update = useUpdateApp();
  const busy = create.isPending || update.isPending;
  const error = create.error ?? update.error;
  const save = () => {
    const input = { name, url, logo: logo || guessLogo(url) };
    const done = { onSuccess: onClose };
    if (app) update.mutate({ id: app.id, input }, done);
    else create.mutate(input, done);
  };
  return (
    <Modal onClose={onClose} label={app ? 'Edit app' : 'Add app'} width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <h3>{app ? 'Edit app' : 'Add an app'}</h3>
        <label className="ph-field">
          <span>Name</span>
          <input className="sc-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Runbooks" autoFocus maxLength={100} />
        </label>
        <label className="ph-field">
          <span>URL</span>
          <input className="sc-input" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://wiki.example.com/runbooks" />
        </label>
        <div className="ph-field">
          <span>Logo</span>
          <div className="ph-logo-picker" role="listbox" aria-label="Logo">
            <button type="button" role="option" aria-selected={!logo} className={`ph-logo-option${!logo ? ' selected' : ''}`} onClick={() => setLogo('')} title="Automatic">
              <AppWindow size={18} />
            </button>
            {APP_LOGOS.map((l) => (
              <button
                type="button"
                role="option"
                aria-selected={logo === l.key}
                key={l.key}
                className={`ph-logo-option${logo === l.key ? ' selected' : ''}`}
                onClick={() => setLogo(l.key)}
                title={l.name}
              >
                <IntegrationLogo logo={l.key} name={l.name} size={18} />
              </button>
            ))}
          </div>
        </div>
        {error && <div className="ph-form-error">{error.message}</div>}
        <div className="ph-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={busy || !name.trim() || !url.trim()}>
            {app ? 'Save' : 'Add App'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Home → Apps: shortcuts to the org's tools. */
export function HomeApps({ canEdit }: { canEdit: boolean }) {
  const q = useApps();
  const remove = useDeleteApp();
  const [editing, setEditing] = useState<PortalApp | 'new' | null>(null);
  const apps = q.data ?? [];
  return (
    <HomeSection
      id="apps"
      icon={<LayoutGrid size={16} />}
      title="Apps"
      actions={
        canEdit &&
        apps.length > 0 && (
          <button className="btn btn-sm" onClick={() => setEditing('new')}>
            <Plus size={13} /> Add App
          </button>
        )
      }
    >
      {q.isLoading ? (
        <Loading />
      ) : q.error ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : apps.length === 0 ? (
        <div className="ph-apps-empty">
          <Mosaic />
          <div className="ph-apps-card">
            <span className="ph-apps-icon">
              <LayoutGrid size={20} />
            </span>
            <strong>Access your tools from the portal</strong>
            <p>Add the tools your teams use every day (repositories, wikis, runbooks, feature flags) so they are one click away.</p>
            {canEdit ? (
              <button className="btn btn-primary" onClick={() => setEditing('new')}>
                <Plus size={14} /> Add App
              </button>
            ) : (
              <span className="sc-muted">Ask a standard user or an admin to add apps.</span>
            )}
          </div>
        </div>
      ) : (
        <div className="ph-app-grid">
          {apps.map((a) => (
            <div key={a.id} className="ph-app-tile">
              <a href={a.url} target="_blank" rel="noreferrer noopener" className="ph-app-link">
                <span className="ph-app-logo">{a.logo ? <IntegrationLogo logo={a.logo} name={a.name} size={26} /> : <AppWindow size={24} />}</span>
                <span className="ph-app-name">{a.name}</span>
                <ExternalLink size={12} className="ph-app-ext" />
              </a>
              {canEdit && (
                <span className="ph-app-tools">
                  <button className="sc-icon-btn" aria-label={`Edit ${a.name}`} onClick={() => setEditing(a)}>
                    <Pencil size={13} />
                  </button>
                  <button className="sc-icon-btn" aria-label={`Remove ${a.name}`} onClick={() => remove.mutate(a.id)}>
                    <Trash2 size={13} />
                  </button>
                </span>
              )}
            </div>
          ))}
        </div>
      )}
      {editing && <AppModal app={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </HomeSection>
  );
}

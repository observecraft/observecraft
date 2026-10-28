import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ChevronRight, Flag, Pencil, Search, Trash2, X } from 'lucide-react';
import type { FeatureFlagEnvironment } from '../../api/featureFlags/client';
import { listIngestKeys } from '../../api/ingest/client';
import { useQuery } from '@tanstack/react-query';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useDeleteEnvironment, useFlagEnvironments, useSaveEnvironment } from './queries';
import '../settings/settings.css';
import '../monitors/monitors.css';
import './featureFlags.css';

const NAV: { label: string; to?: string }[] = [
  { label: 'Environments', to: '/feature-flags/environments' },
  { label: 'Automations' },
  { label: 'Saved Filters' },
  { label: 'Targeting Attributes' },
  { label: 'Stale Flag Detection' },
];

/** Feature Flags → Settings: the environments SDK evaluations run in. */
export function EnvironmentsPage() {
  const q = useFlagEnvironments();
  const [text, setText] = useState('');
  const [editing, setEditing] = useState<FeatureFlagEnvironment | 'new' | null>(null);
  const del = useDeleteEnvironment();
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = (q.data ?? []).filter((e) => words.every((w) => e.name.toLowerCase().includes(w)));
  const remove = (e: FeatureFlagEnvironment) =>
    window.confirm(`Delete ${e.name}? Every flag's targeting in it is deleted too, and SDKs sending its env values get INVALID_CONTEXT.`) && del.mutate(e.id);
  return (
    <div className="settings-page">
      <header className="logs-config-header mon-crumbs">
        <Link to="/feature-flags" className="mon-crumb-root">
          <Flag size={22} /> Feature Flags
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Settings</span>
      </header>
      <div className="settings-body org-body">
        <aside className="settings-nav org-nav" aria-label="Feature flag settings">
          <div className="org-nav-group">
            {NAV.map((i) =>
              i.to ? (
                <Link key={i.label} to={i.to} className="settings-link active">
                  {i.label}
                </Link>
              ) : (
                <span key={i.label} className="settings-link settings-link-soon" title={`${i.label} isn't built yet`}>
                  {i.label}
                  <span className="badge-soon">SOON</span>
                </span>
              ),
            )}
          </div>
        </aside>
        <main className="ff-settings-main">
          <h2>Environments</h2>
          <p>
            An environment groups the runtime <code>env</code> values your applications report, such as <code>production</code>, <code>staging-eu</code> or <code>dev-local</code>. Each flag is turned
            on and targeted per environment; an SDK evaluation lands in the environment whose queries match the <code>env</code> (or <code>environment</code>) attribute of its evaluation context, or
            in its API key’s default environment when the context has none. Use <code>*</code> to match several values.
          </p>
          <div className="ff-settings-bar">
            <div className="mon-filter">
              <Search size={16} />
              <input placeholder="Search by environment name" value={text} onChange={(e) => setText(e.target.value)} />
            </div>
            <button className="btn btn-primary" onClick={() => setEditing('new')}>
              Create Environment
            </button>
          </div>
          {q.isPending ? (
            <Loading label="Loading environments…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table ff-table">
                <thead>
                  <tr>
                    <th>NAME</th>
                    <th style={{ width: 130 }}>PRODUCTION?</th>
                    <th style={{ width: 210 }}>REQUIRES FLAG APPROVAL?</th>
                    <th>QUERIES</th>
                    <th style={{ width: 80 }} />
                  </tr>
                </thead>
                <tbody>
                  {shown.map((e) => (
                    <tr key={e.id} onClick={() => setEditing(e)}>
                      <td>
                        <b>{e.name}</b>
                        {e.apiKeys.length > 0 && <div className="ff-key">Default for {e.apiKeys.map((k) => k.name).join(', ')}</div>}
                      </td>
                      <td>{e.isProduction ? 'Yes' : 'No'}</td>
                      <td>{e.requiresApproval ? 'Yes' : 'No'}</td>
                      <td>
                        <span className="ff-queries">{e.queries.length ? e.queries.map((qv) => <code key={qv}>{qv}</code>) : <span className="muted">None (API key default only)</span>}</span>
                      </td>
                      <td onClick={(ev) => ev.stopPropagation()}>
                        <button className="icon-plain" title="Edit" onClick={() => setEditing(e)}>
                          <Pencil size={15} />
                        </button>{' '}
                        <button className="icon-plain" title="Delete" onClick={() => remove(e)}>
                          <Trash2 size={15} />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {shown.length === 0 && (
                <div className="ff-no-results">
                  <Search size={64} strokeWidth={1.2} />
                  No matching results found
                </div>
              )}
            </div>
          )}
        </main>
      </div>
      {editing && <EnvironmentForm env={editing === 'new' ? undefined : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function EnvironmentForm({ env, onClose }: { env?: FeatureFlagEnvironment; onClose: () => void }) {
  const [name, setName] = useState(env?.name ?? '');
  const [queries, setQueries] = useState((env?.queries ?? []).join(', '));
  const [isProduction, setProduction] = useState(env?.isProduction ?? false);
  const [requiresApproval, setApproval] = useState(env?.requiresApproval ?? false);
  const [keyIds, setKeyIds] = useState<string[]>(() => (env?.apiKeys ?? []).map((k) => k.id));
  const keys = useQuery({ queryKey: ['ingest-keys'], queryFn: listIngestKeys, retry: 0 });
  const save = useSaveEnvironment(env?.id);
  const active = (keys.data ?? []).filter((k) => !k.revokedAt);
  const list = queries
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);
  const submit = () =>
    save.mutate(
      { name, queries: list.length || env ? list : [name.trim().toLowerCase().replace(/\s+/g, '-')], isProduction, requiresApproval, apiKeyIds: keys.data ? keyIds : undefined },
      { onSuccess: onClose },
    );
  return (
    <Modal onClose={onClose} label={env ? 'Edit environment' : 'Create environment'} width={620} height="auto">
      <div className="small-modal ff-env-form">
        <div className="modal-title-row">
          <h3>{env ? `Edit ${env.name}` : 'Create Environment'}</h3>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="ff-field">
          <span>
            Name <em>*</em>
          </span>
          <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Staging EU" maxLength={100} />
        </label>
        <label className="ff-field">
          <span>Queries</span>
          <input value={queries} onChange={(e) => setQueries(e.target.value)} placeholder={name ? name.trim().toLowerCase().replace(/\s+/g, '-') : 'staging-eu, staging-*'} />
          <small>The env values that select this environment, comma separated; * matches any characters. Empty on create: the name in lowercase.</small>
        </label>
        <label className="ff-check">
          <input type="checkbox" checked={isProduction} onChange={(e) => setProduction(e.target.checked)} /> Production environment
        </label>
        <label className="ff-check">
          <input type="checkbox" checked={requiresApproval} onChange={(e) => setApproval(e.target.checked)} /> Requires flag approval (recorded; approvals aren’t enforced yet)
        </label>
        <div className="ff-field">
          <span>Default for API keys</span>
          <small>Evaluations sent with these keys use this environment when their context has no env. Changing them needs an admin.</small>
          {keys.isError ? (
            <small>API keys are visible to admins only.</small>
          ) : (
            <div className="ff-key-list">
              {active.length === 0 && <span className="muted">No API keys.</span>}
              {active.map((k) => (
                <label key={k.id}>
                  <input type="checkbox" checked={keyIds.includes(k.id)} onChange={(e) => setKeyIds((ids) => (e.target.checked ? [...ids, k.id] : ids.filter((x) => x !== k.id)))} />
                  {k.name} <span className="ff-key">{k.prefix}…</span>
                </label>
              ))}
            </div>
          )}
        </div>
        {save.isError && <span className="ff-error-text">{(save.error as Error).message}</span>}
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending} onClick={submit}>
            {env ? 'Save' : 'Create Environment'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

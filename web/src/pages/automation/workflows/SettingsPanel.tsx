import { useState } from 'react';
import { Copy, KeyRound, Plus, Trash2, Undo2, X } from 'lucide-react';
import type { Workflow } from '../../../api/workflows/types';
import { useCreateWorkflowToken } from '../queries';
import type { Draft } from './draft';

interface Props {
  workflow: Workflow;
  draft: Draft;
  onChange: (d: Draft) => void;
  onClose: () => void;
}

/** Right-side panel: the workflow's details, secrets and API token. */
export function SettingsPanel({ workflow, draft, onChange, onClose }: Props) {
  const token = useCreateWorkflowToken();
  const [shown, setShown] = useState<string | null>(null);
  const [newSecret, setNewSecret] = useState({ name: '', value: '' });
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  const secretNames = [...new Set([...workflow.secretNames, ...Object.keys(draft.secrets)])].sort();
  const origin = typeof window !== 'undefined' ? window.location.origin : '';
  const curl = `curl -X POST ${origin}/api/v1/workflows/${workflow.id}/trigger \\\n  -H "Authorization: Bearer $OC_APP_KEY" \\\n  -H "X-Workflow-Token: ${shown ?? '$OC_WORKFLOW_TOKEN'}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"inputs": {}}'`;
  return (
    <aside className="wfe-panel" onClick={(e) => e.stopPropagation()}>
      <div className="wfe-panel-head">
        <div className="wfe-panel-title">
          <div>Workflow settings</div>
          <div className="wf-muted">Saved with the workflow</div>
        </div>
        <button className="wf-icon-btn" aria-label="Close" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="wfe-panel-body">
        <label className="wf-field">
          <span className="wf-field-label">Description</span>
          <textarea className="wf-input" rows={3} value={draft.description} onChange={(e) => set({ description: e.target.value })} />
        </label>
        <label className="wf-field">
          <span className="wf-field-label">Handle</span>
          <input className="wf-input mono" value={draft.handle} onChange={(e) => set({ handle: e.target.value.toLowerCase() })} />
          <span className="wf-field-hint">
            Monitors start it by mentioning <span className="mono">@workflow-{draft.handle || '…'}</span> in their message.
          </span>
        </label>
        <label className="wf-field">
          <span className="wf-field-label">Tags</span>
          <input
            className="wf-input"
            value={draft.tags.join(', ')}
            placeholder="team:checkout, service:order-service"
            onChange={(e) =>
              set({
                tags: e.target.value
                  .split(',')
                  .map((t) => t.trim())
                  .filter(Boolean),
              })
            }
          />
        </label>
        <div className="wfp-row">
          <label className="wf-field wfp-grow">
            <span className="wf-field-label">Owner team</span>
            <input className="wf-input" value={draft.ownerTeam} placeholder="checkout" onChange={(e) => set({ ownerTeam: e.target.value })} />
          </label>
          <label className="wf-field wfp-grow">
            <span className="wf-field-label">Service</span>
            <input className="wf-input" value={draft.service} placeholder="order-service" onChange={(e) => set({ service: e.target.value })} />
          </label>
        </div>
        <div className="wft-section">Secrets</div>
        <p className="wf-muted wft-help">
          HTTP requests read them as <span className="mono">{'{{secrets.<name>}}'}</span>. Values are sealed on the server, never shown again and redacted from runs.
        </p>
        {secretNames.map((n) => {
          const change = draft.secrets[n];
          const removed = change === null;
          return (
            <div key={n} className={`wfs-secret${removed ? ' removed' : ''}`}>
              <KeyRound size={13} />
              <span className="mono">{n}</span>
              <span className="wf-muted">{removed ? 'will be removed' : change !== undefined ? 'new value, unsaved' : 'stored'}</span>
              <span className="wfi-spacer" />
              {change !== undefined ? (
                <button
                  className="wf-icon-btn"
                  aria-label={`Undo ${n}`}
                  onClick={() => {
                    const next = { ...draft.secrets };
                    delete next[n];
                    set({ secrets: next });
                  }}
                >
                  <Undo2 size={13} />
                </button>
              ) : (
                <button className="wf-icon-btn" aria-label={`Remove ${n}`} onClick={() => set({ secrets: { ...draft.secrets, [n]: null } })}>
                  <Trash2 size={13} />
                </button>
              )}
            </div>
          );
        })}
        <div className="wfp-row">
          <input className="wf-input mono" placeholder="name" value={newSecret.name} onChange={(e) => setNewSecret({ ...newSecret, name: e.target.value.toLowerCase() })} />
          <input className="wf-input wfp-grow" type="password" placeholder="value" value={newSecret.value} onChange={(e) => setNewSecret({ ...newSecret, value: e.target.value })} />
          <button
            className="btn btn-sm"
            disabled={!/^[a-z0-9_]{1,40}$/.test(newSecret.name) || !newSecret.value}
            onClick={() => {
              set({ secrets: { ...draft.secrets, [newSecret.name]: newSecret.value } });
              setNewSecret({ name: '', value: '' });
            }}
          >
            <Plus size={13} /> Set
          </button>
        </div>
        <div className="wft-section">API trigger</div>
        <p className="wf-muted wft-help">
          With an API trigger, scripts start the published workflow with an application key and this workflow’s token.{' '}
          {workflow.hasApiToken ? 'A token exists; creating a new one replaces it.' : 'No token yet.'}
        </p>
        <button className="btn btn-sm" disabled={token.isPending} onClick={() => token.mutate(workflow.id, { onSuccess: (t) => setShown(t.token) })}>
          <KeyRound size={13} /> {workflow.hasApiToken ? 'Create a new token' : 'Create a token'}
        </button>
        {token.error && <div className="wf-error">{(token.error as Error).message}</div>}
        {shown && (
          <div className="wfs-token">
            <div className="wf-field-hint">Copy it now: it isn’t shown again.</div>
            <div className="wfp-row">
              <code className="mono wfs-token-value">{shown}</code>
              <button className="wf-icon-btn" aria-label="Copy the token" onClick={() => navigator.clipboard?.writeText(shown)}>
                <Copy size={13} />
              </button>
            </div>
          </div>
        )}
        <pre className="wfs-curl mono">{curl}</pre>
      </div>
    </aside>
  );
}

import { useState } from 'react';
import { Plus, Trash2, Wand2, X } from 'lucide-react';
import type { ApmDefaults, ApmDefaultsInput } from '../../../api/apmSettings/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { useApmDefaults, useUpdateDefaults } from './queries';

/** APM → Settings → Default Settings: the default environment and the primary tags. */
export function ApmDefaultSettingsPage() {
  const q = useApmDefaults();
  const save = useUpdateDefaults();
  const [adding, setAdding] = useState(false);
  if (q.isLoading) return <Loading />;
  if (q.isError || !q.data) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const d = q.data;
  const update = (patch: Partial<ApmDefaultsInput>) => save.mutate({ defaultEnv: d.defaultEnv ?? null, primaryTags: d.primaryTags, defaultPrimaryTag: d.defaultPrimaryTag ?? null, ...patch });
  const env = d.defaultEnv ?? d.detectedEnv ?? '';
  return (
    <div className="settings-pad aps-defaults">
      <h1 className="aps-page-title">Default Settings</h1>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
      <section className="aps-section">
        <h2>APM Default Environment</h2>
        <p>APM pages and queries are scoped to this environment until you pick another one. It starts as the environment with the most active services.</p>
        <p>Choose a different environment here to override it.</p>
        <div className="aps-env-row">
          <select className="aps-select" value={env} onChange={(e) => update({ defaultEnv: e.target.value || null })} aria-label="Default environment">
            {!env && <option value="">No environment seen yet</option>}
            {d.envs.map((e) => (
              <option key={e.env} value={e.env}>
                env:{e.env} ({e.services} services)
              </option>
            ))}
          </select>
          {d.defaultEnv && (
            <button className="link-btn aps-revert" onClick={() => update({ defaultEnv: null })}>
              <Wand2 size={14} /> Revert to Automatically Detected{d.detectedEnv ? ` (env:${d.detectedEnv})` : ''}
            </button>
          )}
          {!d.defaultEnv && d.detectedEnv && <span className="muted">Automatically detected</span>}
        </div>
      </section>
      <section className="aps-section">
        <h2>APM Primary Tags</h2>
        <div className="aps-info">
          <p>
            A primary tag adds a dimension to APM metrics, so request, error and latency series can be split by it. Host and container tags are supported; pick tags with few values, such as{' '}
            <code>availability-zone</code> or <code>datacenter</code>.
          </p>
          <p>Changing primary tags changes how existing APM metrics are grouped: review the dashboards and monitors that use them first.</p>
        </div>
        <h3>Primary Tags</h3>
        <table className="aps-table">
          <thead>
            <tr>
              <th>TAG NAME</th>
              <th className="aps-center">REMOVE</th>
            </tr>
          </thead>
          <tbody>
            {d.primaryTags.map((t) => (
              <tr key={t}>
                <td className="mono">{t}</td>
                <td className="aps-center">
                  <button
                    className="icon-plain"
                    title={`Remove ${t}`}
                    onClick={() => update({ primaryTags: d.primaryTags.filter((x) => x !== t), defaultPrimaryTag: d.defaultPrimaryTag === t ? null : d.defaultPrimaryTag })}
                  >
                    <Trash2 size={15} />
                  </button>
                </td>
              </tr>
            ))}
            {d.primaryTags.length === 0 && (
              <tr>
                <td colSpan={2}>
                  <div className="aps-empty">
                    <h4>You have no primary tags set up</h4>
                    <p className="muted">Primary tags split your APM metrics into finer groups.</p>
                    <button className="btn btn-primary" onClick={() => setAdding(true)}>
                      <Plus size={14} /> Add a Primary Tag
                    </button>
                  </div>
                </td>
              </tr>
            )}
          </tbody>
        </table>
        {d.primaryTags.length > 0 && d.primaryTags.length < 3 && (
          <button className="btn aps-add-below" onClick={() => setAdding(true)}>
            <Plus size={14} /> Add a Primary Tag
          </button>
        )}
        <h3>Default Primary Tag</h3>
        <p>The default primary tag is used across the product to:</p>
        <ul className="aps-bullets">
          <li>scope the Software Catalog</li>
          <li>filter the deployments of a service page</li>
          <li>group Watchdog insights</li>
        </ul>
        <select
          className="aps-select aps-select-sm"
          value={d.defaultPrimaryTag ?? ''}
          disabled={d.primaryTags.length === 0}
          onChange={(e) => update({ defaultPrimaryTag: e.target.value || null })}
          aria-label="Default primary tag"
        >
          <option value="">Select value</option>
          {d.primaryTags.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>
      </section>
      {adding && <AddPrimaryTag d={d} onClose={() => setAdding(false)} onAdd={(t) => update({ primaryTags: [...d.primaryTags, t] })} />}
    </div>
  );
}

function AddPrimaryTag({ d, onClose, onAdd }: { d: ApmDefaults; onClose: () => void; onAdd: (tag: string) => void }) {
  const [tag, setTag] = useState('');
  const options = d.tagCandidates.filter((t) => !d.primaryTags.includes(t));
  return (
    <Modal onClose={onClose} label="Add a primary tag" width={520} height="auto">
      <form
        className="small-modal"
        onSubmit={(e) => {
          e.preventDefault();
          if (tag.trim()) onAdd(tag.trim().toLowerCase());
          onClose();
        }}
      >
        <div className="modal-title-row">
          <h3>Add a Primary Tag</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="aps-field">
          Tag name
          <input list="aps-tag-candidates" value={tag} onChange={(e) => setTag(e.target.value)} placeholder="availability-zone" autoFocus />
          <datalist id="aps-tag-candidates">
            {options.map((t) => (
              <option key={t} value={t} />
            ))}
          </datalist>
        </label>
        <p className="muted">Up to three primary tags. Lowercase letters, digits and _ - . / only.</p>
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!tag.trim()}>
            Add
          </button>
        </div>
      </form>
    </Modal>
  );
}

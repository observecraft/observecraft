import { useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ChevronDown, ChevronRight, Info, KeyRound, Lock, Pencil, Plus, Radar, Trash2 } from 'lucide-react';
import type { SyntheticDefaults, SyntheticSettings, SyntheticVariable } from '../../api/synthetics/client';
import { FREQUENCIES, LOCATION_ID } from '../../api/synthetics/logic';
import { validVariableName } from '../../api/synthetics/model';
import { ConfigLayout, ConfigNotAvailable, type ConfigNavGroup } from '../../components/shell/ConfigLayout';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { TagChips } from '../monitors/common';
import { useDeleteSyntheticVariable, useSaveSyntheticSettings, useSaveSyntheticVariable, useSyntheticLocations, useSyntheticSettings, useSyntheticVariables } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';
import './syntheticsProduct.css';

const NAV: ConfigNavGroup[] = [
  {
    section: 'SYNTHETIC MONITORING',
    items: [
      { to: '/synthetics/settings/default', label: 'Default Settings' },
      { to: '/synthetics/settings/profiles', label: 'Profiles' },
      { to: '/synthetics/settings/downtimes', label: 'Downtimes' },
      { to: '/synthetics/settings/variables', label: 'Global Variables' },
      { to: '/synthetics/settings/integrations', label: 'Integration Settings' },
      { to: '/synthetics/settings/private-locations', label: 'Private Locations' },
    ],
  },
  { section: 'CONTINUOUS TESTING', items: [{ to: '/synthetics/settings/continuous-testing', label: 'Default Settings' }] },
  { section: 'MOBILE APPLICATION TESTING', items: [{ to: '/synthetics/settings/mobile-applications', label: 'Mobile Applications' }] },
];

/** Synthetic Monitoring & Testing › Settings. */
export function SyntheticsSettingsLayout() {
  return <ConfigLayout root={{ to: '/synthetics', label: 'Synthetic Monitoring & Testing', icon: <Radar size={22} /> }} title="Settings" nav={NAV} label="Synthetic Monitoring & Testing settings" />;
}

const DEFAULTS: SyntheticDefaults = { locations: [LOCATION_ID], tags: [], timeoutSeconds: 30, frequencySeconds: 300, retry: { count: 0, intervalMs: 300 }, minFailureDurationMinutes: 0 };

/** The settings with every field present. */
export function fullSettings(s: SyntheticSettings | undefined): Required<SyntheticSettings> {
  return { enforceTagKeys: !!s?.enforceTagKeys, enforceEnv: !!s?.enforceEnv, defaults: { ...DEFAULTS, ...s?.defaults } };
}

const list = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

// ---------------------------------------------------------------- Default Settings

export function DefaultSettingsPage() {
  const q = useSyntheticSettings();
  if (q.isPending) return <Loading label="Loading settings…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <DefaultSettingsForm settings={fullSettings(q.data)} />;
}

function DefaultSettingsForm({ settings }: { settings: Required<SyntheticSettings> }) {
  const [enforceTagKeys, setEnforceTagKeys] = useState(settings.enforceTagKeys);
  const [enforceEnv, setEnforceEnv] = useState(settings.enforceEnv);
  const [d, setD] = useState<SyntheticDefaults>(settings.defaults);
  const [tags, setTags] = useState(settings.defaults.tags.join(', '));
  const [open, setOpen] = useState<string | null>('locations');
  const saveTags = useSaveSyntheticSettings();
  const saveForm = useSaveSyntheticSettings();
  const locations = useSyntheticLocations();
  const set = (p: Partial<SyntheticDefaults>) => setD((x) => ({ ...x, ...p }));
  const base = (): SyntheticSettings => ({ enforceTagKeys: settings.enforceTagKeys, enforceEnv: settings.enforceEnv, defaults: settings.defaults });
  const keys = [...new Set(list(tags).map((t) => (t.includes(':') ? t.slice(0, t.indexOf(':')) : t)))];

  const section = (id: string, title: string, children: ReactNode, hint?: string) => (
    <div className="syn-accordion-item">
      <button className="syn-accordion-head" onClick={() => setOpen((o) => (o === id ? null : id))} aria-expanded={open === id}>
        {open === id ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
        <b>{title}</b>
        {hint && <span className="muted">{hint}</span>}
      </button>
      {open === id && <div className="syn-accordion-body">{children}</div>}
    </div>
  );

  return (
    <div className="settings-pad syn-settings">
      <h1 className="settings-title">Default Settings</h1>
      <section className="settings-card">
        <h2>Enforced Tags Settings</h2>
        <label className="syn-check">
          <input type="checkbox" className="checkbox" checked={enforceTagKeys} onChange={(e) => setEnforceTagKeys(e.target.checked)} />
          <span>
            Require a tag for each key of the default tags on every test
            {keys.length > 0 && <span className="muted"> (now: {keys.join(', ')})</span>}
          </span>
        </label>
        <label className="syn-check">
          <input type="checkbox" className="checkbox" checked={enforceEnv} onChange={(e) => setEnforceEnv(e.target.checked)} />
          <span>Require an environment (env) on every test</span>
        </label>
        <p className="syn-note">
          <Info size={14} /> The rules apply when a test is created or its tags or environment change; existing tests keep running. Set the tag keys under Default Form Settings › Default Tags.
        </p>
        {saveTags.isError && <p className="form-error">{(saveTags.error as Error).message}</p>}
        <div className="syn-form-actions">
          <button className="btn btn-primary" disabled={saveTags.isPending} onClick={() => saveTags.mutate({ ...base(), enforceTagKeys, enforceEnv })}>
            {saveTags.isPending ? 'Saving…' : 'Save'}
          </button>
          {saveTags.isSuccess && <span className="muted">Saved.</span>}
        </div>
      </section>

      <section className="settings-card">
        <h2>Default Form Settings</h2>
        <p className="muted">What a new test starts with in the editor. Each test can change them.</p>
        <div className="syn-accordion">
          {section(
            'locations',
            'Default Locations',
            <>
              {(locations.data ?? [{ id: LOCATION_ID, name: 'ObserveCraft server', privateTargets: false }]).map((l) => (
                <label key={l.id} className="syn-check">
                  <input type="checkbox" className="checkbox" checked={d.locations.includes(l.id)} disabled title="Tests run from the ObserveCraft server" />
                  <span>{l.name}</span>
                </label>
              ))}
              <p className="muted">Tests run from the ObserveCraft server. Managed and private locations aren't available yet.</p>
            </>,
            `${d.locations.length} location`,
          )}
          {section('browsers', 'Default Browsers & Devices', <p className="muted">Browser tests aren't available yet, so there are no browsers or devices to choose.</p>, 'not available yet')}
          {section(
            'tags',
            'Default Tags',
            <label className="form-label">
              Tags
              <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="team:, service:storefront-api" />
              <span className="muted">A key with no value (team:) names a key every test must have when the rule above is on.</span>
            </label>,
            list(tags).length ? list(tags).join(', ') : 'none',
          )}
          {section(
            'timeouts',
            'Default Timeouts',
            <label className="form-label syn-narrow">
              Request timeout (seconds)
              <input className="syn-input" type="number" min={1} max={60} value={d.timeoutSeconds} onChange={(e) => set({ timeoutSeconds: Number(e.target.value) })} />
            </label>,
            `${d.timeoutSeconds} s`,
          )}
          {section(
            'frequencies',
            'Default Frequencies',
            <div className="syn-freqs" role="radiogroup" aria-label="Default frequency">
              {FREQUENCIES.map((f) => (
                <button
                  key={f.seconds}
                  role="radio"
                  aria-checked={d.frequencySeconds === f.seconds}
                  className={`syn-type-btn${d.frequencySeconds === f.seconds ? ' active' : ''}`}
                  onClick={() => set({ frequencySeconds: f.seconds })}
                >
                  {f.label}
                </button>
              ))}
            </div>,
            `every ${FREQUENCIES.find((f) => f.seconds === d.frequencySeconds)?.label ?? `${d.frequencySeconds}s`}`,
          )}
          {section(
            'retries',
            'Default Retries',
            <div className="syn-row-fields">
              <label className="form-label">
                Retries of a failed run
                <select className="syn-input" value={d.retry.count ?? 0} onChange={(e) => set({ retry: { ...d.retry, count: Number(e.target.value) } })}>
                  {[0, 1, 2, 3].map((n) => (
                    <option key={n} value={n}>
                      {n === 0 ? 'No retry' : `${n} time${n === 1 ? '' : 's'}`}
                    </option>
                  ))}
                </select>
              </label>
              <label className="form-label">
                Between retries (ms)
                <input
                  className="syn-input"
                  type="number"
                  min={0}
                  max={60000}
                  step={100}
                  value={d.retry.intervalMs ?? 300}
                  onChange={(e) => set({ retry: { ...d.retry, intervalMs: Number(e.target.value) } })}
                />
              </label>
              <label className="form-label">
                Alert after failing for (minutes)
                <input className="syn-input" type="number" min={0} max={120} value={d.minFailureDurationMinutes} onChange={(e) => set({ minFailureDurationMinutes: Number(e.target.value) })} />
              </label>
            </div>,
            `${d.retry.count ?? 0} retr${d.retry.count === 1 ? 'y' : 'ies'}`,
          )}
          {section('mobile', 'Default Mobile Devices', <p className="muted">Mobile application tests aren't available yet.</p>, 'not available yet')}
        </div>
        {saveForm.isError && <p className="form-error">{(saveForm.error as Error).message}</p>}
        <div className="syn-form-actions">
          <button className="btn btn-primary" disabled={saveForm.isPending} onClick={() => saveForm.mutate({ ...base(), defaults: { ...d, tags: list(tags) } })}>
            {saveForm.isPending ? 'Saving…' : 'Save Default Form Settings'}
          </button>
          {saveForm.isSuccess && <span className="muted">Saved. New tests start with these settings.</span>}
        </div>
      </section>
    </div>
  );
}

// ---------------------------------------------------------------- Global Variables

export function GlobalVariablesPage() {
  const q = useSyntheticVariables();
  const remove = useDeleteSyntheticVariable();
  const [editing, setEditing] = useState<SyntheticVariable | 'new' | null>(null);
  return (
    <div className="settings-pad syn-settings">
      <div className="settings-title-row">
        <h1 className="settings-title">Global Variables</h1>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={15} /> New Global Variable
        </button>
      </div>
      <p className="muted">
        Values tests share: write <code>{'{{ NAME }}'}</code> in a test's URL, host, headers, body, credentials or assertion targets and it's replaced when the test runs. Secure values can't be read
        back and are masked in results.
      </p>
      {q.isPending ? (
        <Loading label="Loading variables…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <KeyRound size={30} strokeWidth={1.4} />
          <p className="muted">No global variable yet. Add one for a host, a token or a test user's credentials.</p>
        </section>
      ) : (
        <div className="mon-table-wrap">
          <table className="mon-table syn-table syn-static syn-vars">
            <thead>
              <tr>
                <th>NAME</th>
                <th>VALUE</th>
                <th>DESCRIPTION</th>
                <th className="col-tags">TAGS</th>
                <th>USED BY</th>
                <th className="syn-col-modified">LAST MODIFIED</th>
                <th className="syn-col-actions" />
              </tr>
            </thead>
            <tbody>
              {q.data.map((v) => (
                <tr key={v.id}>
                  <td className="mono">{v.name}</td>
                  <td className="mono">
                    {v.secure ? (
                      <span className="muted">
                        <Lock size={12} /> ••••••
                      </span>
                    ) : (
                      <span className="syn-wrap">{v.value}</span>
                    )}
                  </td>
                  <td className="muted">{v.description || '—'}</td>
                  <td className="col-tags">
                    <TagChips tags={v.tags} max={2} />
                  </td>
                  <td>
                    {v.usedBy.length === 0 ? (
                      <span className="muted">No test</span>
                    ) : (
                      v.usedBy.slice(0, 3).map((t) => (
                        <Link key={t.id} to={`/synthetics/tests/${t.id}`} className="syn-chip">
                          {t.name}
                        </Link>
                      ))
                    )}
                    {v.usedBy.length > 3 && <span className="muted"> +{v.usedBy.length - 3}</span>}
                  </td>
                  <td className="muted syn-col-modified">
                    {timeAgo(v.updatedAt)}
                    {v.updatedBy ? ` by ${v.updatedBy}` : ''}
                  </td>
                  <td className="syn-col-actions">
                    <button className="icon-plain" title="Edit" onClick={() => setEditing(v)}>
                      <Pencil size={15} />
                    </button>
                    <button
                      className="icon-plain"
                      title="Delete"
                      onClick={() =>
                        window.confirm(v.usedBy.length ? `${v.usedBy.length} test(s) use ${v.name}; their runs fail until it exists again. Delete it?` : `Delete ${v.name}?`) && remove.mutate(v.id)
                      }
                    >
                      <Trash2 size={15} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {remove.isError && <p className="form-error">{(remove.error as Error).message}</p>}
      {editing && <VariableDialog variable={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function VariableDialog({ variable, onClose }: { variable: SyntheticVariable | null; onClose: () => void }) {
  const save = useSaveSyntheticVariable(variable?.id);
  const [name, setName] = useState(variable?.name ?? '');
  const [value, setValue] = useState(variable?.value ?? '');
  const [secure, setSecure] = useState(variable?.secure ?? false);
  const [description, setDescription] = useState(variable?.description ?? '');
  const [tags, setTags] = useState((variable?.tags ?? []).join(', '));
  const keepSecret = !!variable?.secure && value === '';
  const submit = () => save.mutate({ name, value: keepSecret ? null : value, secure, description, tags: list(tags) }, { onSuccess: onClose });
  const nameOk = validVariableName(name);
  return (
    <Modal onClose={onClose} label={variable ? 'Edit global variable' : 'New global variable'} width={560} height="auto">
      <div className="syn-dialog">
        <h3>{variable ? `Edit ${variable.name}` : 'New Global Variable'}</h3>
        <label className="form-label">
          Name*
          <input className="syn-input mono" value={name} onChange={(e) => setName(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, '_'))} placeholder="CHECKOUT_API_TOKEN" autoFocus />
          <span className="muted">Use it as {nameOk ? `{{ ${name} }}` : '{{ NAME }}'}. Upper-case letters, digits and underscores.</span>
        </label>
        <label className="form-label">
          Value{variable ? '' : '*'}
          <textarea
            className="syn-textarea mono"
            rows={3}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder={variable?.secure ? 'Leave empty to keep the current value' : 'shop.example.com'}
          />
        </label>
        <label className="syn-check">
          <input type="checkbox" className="checkbox" checked={secure} onChange={(e) => setSecure(e.target.checked)} />
          <span>Secure: hide the value once saved and mask it in test results</span>
        </label>
        <label className="form-label">
          Description
          <input className="syn-input" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Token of the synthetic checkout user" />
        </label>
        <label className="form-label">
          Tags
          <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="team:checkout" />
        </label>
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="syn-dialog-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!nameOk || (!variable && value === '') || save.isPending} onClick={submit}>
            {save.isPending ? 'Saving…' : variable ? 'Save' : 'Create'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

// ---------------------------------------------------------------- not available

export function SyntheticsDowntimesSettings() {
  return (
    <ConfigNotAvailable title="Downtimes">
      Each test alerts through its own monitor, so monitor downtimes silence tests: schedule them in <Link to="/monitors/downtimes">Monitors › Downtimes</Link>, scoped to a test's monitor or to its
      tags. Downtimes that pause the test runs themselves aren't available yet.
    </ConfigNotAvailable>
  );
}

export function SyntheticsProfilesSettings() {
  return <ConfigNotAvailable title="Profiles">Profiles of browser settings (cookies, devices, network conditions) are for browser tests, which aren't available yet.</ConfigNotAvailable>;
}

export function SyntheticsIntegrationSettings() {
  return (
    <ConfigNotAvailable title="Integration Settings">
      Every scheduled run already writes <code>synthetics.test_runs</code> and the response time metrics (<code>synthetics.http.response.time</code>, …), tagged with the test's tags, for{' '}
      <Link to="/metrics/explorer">dashboards, monitors and SLOs</Link>. Linking runs to APM traces (trace header injection) isn't available yet.
    </ConfigNotAvailable>
  );
}

export function SyntheticsPrivateLocations() {
  return (
    <ConfigNotAvailable title="Private Locations">
      Tests run from the ObserveCraft server itself. Private locations (workers in your own networks) aren't available yet; to test internal endpoints from the server, the operator can allow private
      targets with <code>OC_SYNTHETICS_ALLOW_PRIVATE=true</code>.
    </ConfigNotAvailable>
  );
}

export function ContinuousTestingSettings() {
  return <ConfigNotAvailable title="Continuous Testing Default Settings">Running tests from CI pipelines (continuous testing) isn't available yet, so it has no settings.</ConfigNotAvailable>;
}

export function MobileApplicationsSettings() {
  return <ConfigNotAvailable title="Mobile Applications">Mobile application testing isn't available yet: there are no applications to upload or devices to run them on.</ConfigNotAvailable>;
}

// Integrations → Google Cloud: the org's service accounts (the rows Terraform's
// `datadog_integration_gcp_sts` manages too), their crawl per project, and the Google Cloud services the
// integration collects. Shares the cloud account layout of the AWS page (aws.css).
import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, ChevronRight, Info, Plus, Trash2, TriangleAlert } from 'lucide-react';
import seed from '../../../../../server/seeds/gcp_integration.json';
import type { GcpAccount, GcpAccountInput, GcpConfig, GcpService } from '../../../api/gcp/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { IntegrationLogo } from '../IntegrationLogo';
import { useGcpAccountMutations, useGcpAccountStatus, useGcpAccounts } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';

const SERVICES = seed.services as GcpService[];
/** User-defined metric domains, collected only once turned on (gcp/naming.rs collected). */
const CUSTOM = seed.custom_namespaces as { prefix: string; short: string; name: string }[];
const DESCRIPTION = 'Google Cloud is a collection of cloud services. The integration collects Cloud Monitoring metrics from the projects a service account can see, which ObserveCraft impersonates.';
const splitList = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

export function GcpPage() {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'services' ? 'services' : 'configuration';
  const accounts = useGcpAccounts();
  const setParam = (key: string, value: string | null) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (value) next.set(key, value);
      else next.delete(key);
      return next;
    });
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Google Cloud Platform</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="googlecloud" name="Google Cloud Platform" size={72} />
        <div>
          <h1>Google Cloud Platform</h1>
          <p>{DESCRIPTION}</p>
        </div>
        {(accounts.data?.length ?? 0) > 0 && (
          <span className="btn aws-installed">
            <Check size={15} /> Installed
          </span>
        )}
      </header>
      <nav className="im-tabs aws-tabs">
        <button className={tab === 'configuration' ? 'active' : ''} onClick={() => setParam('tab', null)}>
          Configuration
        </button>
        <button className={tab === 'services' ? 'active' : ''} onClick={() => setParam('tab', 'services')}>
          Google Cloud Integrations
        </button>
      </nav>
      {accounts.isPending ? (
        <Loading label="Loading service accounts…" />
      ) : accounts.isError ? (
        <ErrorState error={accounts.error} onRetry={() => accounts.refetch()} />
      ) : tab === 'services' ? (
        <Services />
      ) : accounts.data.length === 0 ? (
        <div className="aws-empty">
          <p>Add a service account to collect the Cloud Monitoring metrics of the projects it can see.</p>
          <Link className="btn btn-primary" to="/integrations/gcp/add">
            <Plus size={15} /> Add GCP Account
          </Link>
          <Services />
        </div>
      ) : (
        <Accounts accounts={accounts.data} selected={params.get('account')} onSelect={(id) => setParam('account', id)} />
      )}
    </div>
  );
}

function Services() {
  return (
    <section className="aws-services">
      <h2>Includes {SERVICES.length} Google Cloud integrations</h2>
      <div className="aws-service-grid">
        {SERVICES.map((s) => (
          <div key={s.id} className="aws-service" title={`${s.prefix} · gcp.${s.short}.*`}>
            <IntegrationLogo logo={`tile:${s.id}`} name={s.name} size={28} />
            <span>{s.name}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function Accounts({ accounts, selected, onSelect }: { accounts: GcpAccount[]; selected: string | null; onSelect: (id: string | null) => void }) {
  const current = accounts.find((a) => a.id === selected) ?? accounts[0];
  return (
    <>
      <div className="aws-toolbar">
        <span className="aws-search-spacer" />
        <Link className="btn btn-primary" to="/integrations/gcp/add">
          <Plus size={15} /> Add GCP Account
        </Link>
      </div>
      <div className="aws-split">
        <aside className="aws-account-list">
          <h3>Showing {accounts.length} Service Accounts</h3>
          {accounts.map((a) => (
            <button key={a.id} className={a.id === current.id ? 'selected' : ''} onClick={() => onSelect(a.id)}>
              <strong>{a.client_email.split('@')[0]}</strong>
              <span className="aws-tags">
                <span className="aws-tag">{a.client_email.split('@')[1]?.replace('.iam.gserviceaccount.com', '')}</span>
                {a.config.account_tags.map((t) => (
                  <span key={t} className="aws-tag">
                    {t}
                  </span>
                ))}
              </span>
            </button>
          ))}
        </aside>
        <Detail key={current.id} account={current} onDeleted={() => onSelect(null)} />
      </div>
    </>
  );
}

const TABS = ['Issues', 'Details', 'Metric Collection', 'Resource Collection'] as const;
const ISSUES: Record<string, string> = {
  auth: 'ObserveCraft cannot impersonate this service account: grant this install’s service account the Service Account Token Creator role on it.',
  disabled: 'The Cloud Monitoring API is disabled in this project: enable it, or the project is retried daily.',
  throttled: 'Google is throttling ObserveCraft’s calls; collection retries with backoff.',
};

function Detail({ account, onDeleted }: { account: GcpAccount; onDeleted: () => void }) {
  const [tab, setTab] = useState<(typeof TABS)[number]>('Issues');
  const status = useGcpAccountStatus(account.id);
  const { update, remove } = useGcpAccountMutations();
  const save = (input: GcpAccountInput) => update.mutate({ id: account.id, input });
  const issues = status.data?.projects.filter((p) => p.error) ?? [];
  return (
    <section className="aws-detail">
      <header>
        <h2>{account.client_email}</h2>
      </header>
      <nav className="im-tabs">
        {TABS.map((t) => (
          <button key={t} className={t === tab ? 'active' : ''} onClick={() => setTab(t)}>
            {t}
            {t === 'Issues' && issues.length > 0 && <span className="aws-dot" />}
          </button>
        ))}
      </nav>
      {update.error && <p className="form-error">{(update.error as Error).message}</p>}
      <div className="aws-detail-body">
        {tab === 'Issues' &&
          (status.isPending ? (
            <Loading label="Loading collection status…" />
          ) : status.isError ? (
            <ErrorState error={status.error} onRetry={() => status.refetch()} />
          ) : (
            <div className="aws-form">
              {issues.length === 0 ? (
                <div className="aws-note">
                  <Check size={16} />
                  <span>
                    {status.data.projects.some((p) => p.last_success_at) ? 'No issues: every project is being collected.' : 'Collection starts within a few minutes of adding the service account.'}
                  </span>
                </div>
              ) : (
                issues.map((p) => (
                  <div key={p.scope} className="aws-issue">
                    <div>
                      <span className="aws-warning-badge">{p.error!.kind === 'throttled' ? 'INFO' : 'WARNING'}</span> <strong>{p.scope ? `Project ${p.scope}` : 'Listing the projects'}</strong>{' '}
                      <span className="muted">last seen {timeAgo(p.error!.at)}</span>
                    </div>
                    <p>
                      {p.error!.missing_action ? (
                        <>
                          Grant the service account a role with <code>{p.error!.missing_action}</code> (e.g. Monitoring Viewer) on this project.
                        </>
                      ) : (
                        (ISSUES[p.error!.kind] ?? p.error!.message)
                      )}
                    </p>
                    <small className="muted mono">{p.error!.message}</small>
                  </div>
                ))
              )}
              <table className="im-table">
                <thead>
                  <tr>
                    <th>PROJECT</th>
                    <th>COLLECTED UNTIL</th>
                    <th>SERIES</th>
                    <th>STATUS</th>
                  </tr>
                </thead>
                <tbody>
                  {status.data.projects
                    .filter((p) => p.scope)
                    .map((p) => (
                      <tr key={p.scope}>
                        <td className="mono">{p.scope}</td>
                        <td>{p.collected_until ? timeAgo(p.collected_until) : '—'}</td>
                        <td>{p.metrics}</td>
                        <td>{p.error ? <TriangleAlert size={14} className="aws-warn-icon" /> : p.last_success_at ? <Check size={14} className="aws-ok-icon" /> : 'Pending'}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          ))}
        {tab === 'Details' && <Details account={account} pending={update.isPending} onSave={save} onDelete={() => remove.mutate(account.id, { onSuccess: onDeleted })} deleting={remove.isPending} />}
        {tab === 'Metric Collection' && <Metrics config={account.config} pending={update.isPending} onSave={save} />}
        {tab === 'Resource Collection' && <Resources config={account.config} pending={update.isPending} onSave={save} />}
      </div>
    </section>
  );
}

function Save({ dirty, pending, onSave }: { dirty: boolean; pending: boolean; onSave: () => void }) {
  return (
    <div className="aws-save">
      <button className="btn btn-primary" disabled={!dirty || pending} onClick={onSave}>
        {pending ? 'Saving…' : 'Save'}
      </button>
    </div>
  );
}

function Toggle({ label, hint, value, onChange }: { label: string; hint?: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="aws-toggle">
      <button type="button" role="switch" aria-checked={value} className={`switch${value ? ' on' : ''}`} onClick={() => onChange(!value)} />
      <span>
        {label}
        {hint && <small className="muted">{hint}</small>}
      </span>
    </label>
  );
}

function Details({ account, pending, onSave, onDelete, deleting }: { account: GcpAccount; pending: boolean; onSave: (i: GcpAccountInput) => void; onDelete: () => void; deleting: boolean }) {
  const [tags, setTags] = useState(account.config.account_tags.join(', '));
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="aws-form">
      <dl className="aws-facts">
        <dt>Service account</dt>
        <dd className="mono">{account.client_email}</dd>
        <dt>Access</dt>
        <dd>Impersonated by this install (no key stored)</dd>
      </dl>
      <label className="rt-field">
        <span>Account tags</span>
        <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="key:value, key:value" />
      </label>
      <Save dirty={splitList(tags).join() !== account.config.account_tags.join()} pending={pending} onSave={() => onSave({ account_tags: splitList(tags) })} />
      <div className="aws-danger">
        {confirm ? (
          <>
            <span>Remove this service account? ObserveCraft stops collecting from its projects; it stays in Google Cloud.</span>
            <button className="btn" onClick={() => setConfirm(false)}>
              Cancel
            </button>
            <button className="btn btn-danger" disabled={deleting} onClick={onDelete}>
              {deleting ? 'Removing…' : 'Remove'}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => setConfirm(true)}>
            <Trash2 size={14} /> Remove service account
          </button>
        )}
      </div>
    </div>
  );
}

const RESOURCE_TYPES: [string, string][] = [
  ['gce_instance', 'Compute Engine instances'],
  ['cloud_run_revision', 'Cloud Run revisions'],
  ['cloud_function', 'Cloud Functions'],
];

function Metrics({ config, pending, onSave }: { config: GcpConfig; pending: boolean; onSave: (i: GcpAccountInput) => void }) {
  const filtersOf = (type: string) => (config.monitored_resource_configs.find((c) => c.type === type)?.filters ?? []).join(', ');
  const isCustom = (id: string) => CUSTOM.some((c) => c.short === id);
  const [automute, setAutomute] = useState(config.automute);
  const [quota, setQuota] = useState(config.is_per_project_quota_enabled);
  const [regions, setRegions] = useState(config.region_filter_configs.join(', '));
  const [global, setGlobal] = useState(config.is_global_location_enabled);
  const [filters, setFilters] = useState<Record<string, string>>(Object.fromEntries(RESOURCE_TYPES.map(([t]) => [t, filtersOf(t)])));
  const [disabled, setDisabled] = useState<string[]>(config.metric_namespace_configs.filter((c) => c.disabled && !isCustom(c.id)).map((c) => c.id));
  const [custom, setCustom] = useState<string[]>(config.metric_namespace_configs.filter((c) => !c.disabled && isCustom(c.id)).map((c) => c.id));
  const monitored = RESOURCE_TYPES.map(([type]) => ({ type, filters: splitList(filters[type]) })).filter((c) => c.filters.length);
  const namespaces = [...disabled.map((id) => ({ id, disabled: true, filters: [] })), ...custom.map((id) => ({ id, disabled: false, filters: [] }))];
  const saved = config.metric_namespace_configs
    .filter((c) => (isCustom(c.id) ? !c.disabled : c.disabled))
    .map((c) => `${c.id}:${!!c.disabled}`)
    .sort()
    .join();
  const dirty =
    automute !== config.automute ||
    quota !== config.is_per_project_quota_enabled ||
    global !== config.is_global_location_enabled ||
    splitList(regions).join() !== config.region_filter_configs.join() ||
    JSON.stringify(monitored) !== JSON.stringify(config.monitored_resource_configs.filter((c) => c.filters.length)) ||
    namespaces
      .map((c) => `${c.id}:${c.disabled}`)
      .sort()
      .join() !== saved;
  const switchOf = (name: string, on: boolean, toggle: () => void) => (
    <button type="button" role="switch" aria-label={name} aria-checked={on} className={`switch${on ? ' on' : ''}`} onClick={toggle} />
  );
  return (
    <div className="aws-form">
      <Toggle
        label="Mute monitors for expected instance shutdowns"
        hint="Not applied yet: stored for Terraform and API clients; monitors aren't muted when an instance is stopped or preempted."
        value={automute}
        onChange={setAutomute}
      />
      <Toggle
        label="Bill API calls to each monitored project's quota"
        hint="Not applied yet: stored for Terraform and API clients; Cloud Monitoring calls count against the service account's project."
        value={quota}
        onChange={setQuota}
      />
      <h4 className="aws-subhead">Locations</h4>
      <label className="rt-field">
        <span>Regions, zones or multi-regions</span>
        <input value={regions} onChange={(e) => setRegions(e.target.value)} placeholder="europe-west1, us-central1-a, eu (empty: all)" />
      </label>
      <Toggle label="Collect global resources" hint="Resources without a location, or located in global." value={global} onChange={setGlobal} />
      <h4 className="aws-subhead">Resource filters</h4>
      <div className="aws-note">
        <Info size={16} />
        <span>Comma-separated labels (key:value, * wildcards); only resources with one of them are collected, and none with a label preceded by !.</span>
      </div>
      {RESOURCE_TYPES.map(([type, label]) => (
        <label key={type} className="rt-field">
          <span>{label}</span>
          <input value={filters[type]} onChange={(e) => setFilters((f) => ({ ...f, [type]: e.target.value }))} placeholder="env:prod" />
        </label>
      ))}
      <h4 className="aws-subhead">Services</h4>
      <ul className="aws-source-list aws-namespaces">
        {SERVICES.map((s) => {
          const on = !disabled.includes(s.short);
          return (
            <li key={s.id}>
              <span>
                {s.name} <span className="muted mono">gcp.{s.short}.*</span>
              </span>
              {switchOf(s.name, on, () => setDisabled((d) => (on ? [...d, s.short] : d.filter((x) => x !== s.short))))}
            </li>
          );
        })}
      </ul>
      <h4 className="aws-subhead">User-defined metrics</h4>
      <ul className="aws-source-list aws-namespaces">
        {CUSTOM.map((c) => {
          const on = custom.includes(c.short);
          return (
            <li key={c.short}>
              <span>
                {c.name}{' '}
                <span className="muted mono">
                  {c.prefix}* → gcp.{c.short}.*
                </span>
              </span>
              {switchOf(c.name, on, () => setCustom((d) => (on ? d.filter((x) => x !== c.short) : [...d, c.short])))}
            </li>
          );
        })}
      </ul>
      <Save
        dirty={dirty}
        pending={pending}
        onSave={() =>
          onSave({
            automute,
            is_per_project_quota_enabled: quota,
            is_global_location_enabled: global,
            region_filter_configs: splitList(regions),
            monitored_resource_configs: monitored,
            metric_namespace_configs: namespaces,
          })
        }
      />
    </div>
  );
}

function Resources({ config, pending, onSave }: { config: GcpConfig; pending: boolean; onSave: (i: GcpAccountInput) => void }) {
  const [draft, setDraft] = useState({ resource_collection_enabled: config.resource_collection_enabled, is_cspm_enabled: config.is_cspm_enabled });
  return (
    <div className="aws-form">
      <div className="aws-note">
        <Info size={16} />
        <span>Resource collection isn&apos;t built yet: these settings are stored for Terraform and API clients, and have no effect.</span>
      </div>
      <Toggle
        label="Resource collection"
        value={draft.resource_collection_enabled}
        onChange={(v) => setDraft((d) => ({ ...d, resource_collection_enabled: v, is_cspm_enabled: v && d.is_cspm_enabled }))}
      />
      <Toggle
        label="Cloud Security Management scanning (needs resource collection)"
        value={draft.is_cspm_enabled}
        onChange={(v) => setDraft((d) => ({ ...d, is_cspm_enabled: v && d.resource_collection_enabled }))}
      />
      <Save dirty={draft.resource_collection_enabled !== config.resource_collection_enabled || draft.is_cspm_enabled !== config.is_cspm_enabled} pending={pending} onSave={() => onSave(draft)} />
    </div>
  );
}

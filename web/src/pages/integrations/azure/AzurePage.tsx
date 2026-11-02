// Integrations → Microsoft Azure: the org's app registrations (the rows Terraform's
// `datadog_integration_azure` manages too), their crawl per subscription, and the Azure services the
// integration collects. Shares the cloud account layout of the AWS page (aws.css).
import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, ChevronRight, Info, Plus, Trash2, TriangleAlert } from 'lucide-react';
import seed from '../../../../../server/seeds/azure_integration.json';
import type { AzureAccount, AzureAccountInput, AzureConfig, AzureResourceType } from '../../../api/azure/types';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { IntegrationLogo } from '../IntegrationLogo';
import { useAzureAccountMutations, useAzureAccountStatus, useAzureAccounts } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';

const RESOURCE_TYPES = seed.resource_types as AzureResourceType[];
/** Resource providers of the catalog's types (`Microsoft.Compute`, …). */
const PROVIDERS = [...new Set(RESOURCE_TYPES.map((t) => t.type.split('/')[0]))].sort();

const DESCRIPTION = 'Microsoft Azure is a collection of cloud services. The integration collects Azure Monitor metrics from the subscriptions an app registration can read.';

export function AzurePage() {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'services' ? 'services' : 'configuration';
  const accounts = useAzureAccounts();
  const setParam = (key: string, value: string | null) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (value) next.set(key, value);
      else next.delete(key);
      return next;
    });
  const installed = (accounts.data?.length ?? 0) > 0;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>Microsoft Azure</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="tile:azure" name="Microsoft Azure" size={72} />
        <div>
          <h1>Microsoft Azure</h1>
          <p>{DESCRIPTION}</p>
        </div>
        {installed && (
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
          Azure Integrations
        </button>
      </nav>
      {accounts.isPending ? (
        <Loading label="Loading app registrations…" />
      ) : accounts.isError ? (
        <ErrorState error={accounts.error} onRetry={() => accounts.refetch()} />
      ) : tab === 'services' ? (
        <Services />
      ) : accounts.data.length === 0 ? (
        <div className="aws-empty">
          <p>Add an app registration to collect the Azure Monitor metrics of the subscriptions it can read.</p>
          <Link className="btn btn-primary" to="/integrations/azure/add">
            <Plus size={15} /> Add New App Registration
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
      <h2>Includes {RESOURCE_TYPES.length} Azure integrations</h2>
      <div className="aws-service-grid">
        {RESOURCE_TYPES.map((t) => (
          <div key={t.type} className="aws-service" title={`${t.type} · ${t.prefix}.*`}>
            <IntegrationLogo logo={`tile:${t.id}`} name={t.name} size={28} />
            <span>{t.name}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function Accounts({ accounts, selected, onSelect }: { accounts: AzureAccount[]; selected: string | null; onSelect: (id: string | null) => void }) {
  const current = accounts.find((a) => a.id === selected) ?? accounts[0];
  return (
    <>
      <div className="aws-toolbar">
        <span className="aws-search-spacer" />
        <Link className="btn btn-primary" to="/integrations/azure/add">
          <Plus size={15} /> Add New App Registration
        </Link>
      </div>
      <div className="aws-split">
        <aside className="aws-account-list">
          <h3>Showing {accounts.length} App Registrations</h3>
          {accounts.map((a) => (
            <button key={a.id} className={a.id === current.id ? 'selected' : ''} onClick={() => onSelect(a.id)}>
              <strong className="mono">{a.client_id}</strong>
              <span className="aws-tags">
                <span className="aws-tag">tenant:{a.tenant_name}</span>
              </span>
            </button>
          ))}
          {accounts.length === 0 && <Empty>No app registrations.</Empty>}
        </aside>
        <Detail key={current.id} account={current} onDeleted={() => onSelect(null)} />
      </div>
    </>
  );
}

const TABS = ['Issues', 'Details', 'Metric Collection', 'Filters', 'Resource Collection'] as const;

function Detail({ account, onDeleted }: { account: AzureAccount; onDeleted: () => void }) {
  const [tab, setTab] = useState<(typeof TABS)[number]>('Issues');
  const status = useAzureAccountStatus(account.id);
  const { update, remove } = useAzureAccountMutations();
  const save = (input: AzureAccountInput) => update.mutate({ id: account.id, input });
  const issues = status.data?.subscriptions.filter((s) => s.error) ?? [];
  return (
    <section className="aws-detail">
      <header>
        <h2 className="mono">{account.client_id}</h2>
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
              {!account.has_secret && (
                <div className="aws-issue">
                  <strong>This app registration has no client secret.</strong>
                  <p>ObserveCraft signs in with a client secret; secretless authentication isn&apos;t available. Add a secret under Details.</p>
                </div>
              )}
              {issues.length === 0 && account.has_secret ? (
                <div className="aws-note">
                  <Check size={16} />
                  <span>
                    {status.data.subscriptions.some((s) => s.last_success_at)
                      ? 'No issues: every subscription is being collected.'
                      : 'Collection starts within a few minutes of adding the app registration.'}
                  </span>
                </div>
              ) : (
                issues.map((s) => (
                  <div key={s.scope} className="aws-issue">
                    <div>
                      <span className="aws-warning-badge">WARNING</span> <strong>{s.scope ? `Subscription ${s.scope}` : 'Listing the subscriptions'}</strong>{' '}
                      <span className="muted">last seen {timeAgo(s.error!.at)}</span>
                    </div>
                    <p>
                      {s.error!.missing_action ? (
                        <>
                          Assign a role that allows <code>{s.error!.missing_action}</code> (e.g. Monitoring Reader) to the app registration on this subscription.
                        </>
                      ) : s.error!.kind === 'auth' ? (
                        'The app registration cannot sign in: check the tenant, the client ID and the secret (it may have expired).'
                      ) : (
                        s.error!.message
                      )}
                    </p>
                    <small className="muted mono">{s.error!.message}</small>
                  </div>
                ))
              )}
              <table className="im-table">
                <thead>
                  <tr>
                    <th>SUBSCRIPTION</th>
                    <th>COLLECTED UNTIL</th>
                    <th>RESOURCES</th>
                    <th>STATUS</th>
                  </tr>
                </thead>
                <tbody>
                  {status.data.subscriptions
                    .filter((s) => s.scope)
                    .map((s) => (
                      <tr key={s.scope}>
                        <td className="mono">{s.scope}</td>
                        <td>{s.collected_until ? timeAgo(s.collected_until) : '—'}</td>
                        <td>{s.metrics}</td>
                        <td>{s.error ? <TriangleAlert size={14} className="aws-warn-icon" /> : s.last_success_at ? <Check size={14} className="aws-ok-icon" /> : 'Pending'}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          ))}
        {tab === 'Details' && <Details account={account} pending={update.isPending} onSave={save} onDelete={() => remove.mutate(account.id, { onSuccess: onDeleted })} deleting={remove.isPending} />}
        {tab === 'Metric Collection' && <Metrics config={account.config} pending={update.isPending} onSave={save} />}
        {tab === 'Filters' && <Filters config={account.config} pending={update.isPending} onSave={save} />}
        {tab === 'Resource Collection' && <Resources config={account.config} pending={update.isPending} onSave={save} />}
      </div>
    </section>
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

function Save({ dirty, pending, onSave }: { dirty: boolean; pending: boolean; onSave: () => void }) {
  return (
    <div className="aws-save">
      <button className="btn btn-primary" disabled={!dirty || pending} onClick={onSave}>
        {pending ? 'Saving…' : 'Save'}
      </button>
    </div>
  );
}

function Details({ account, pending, onSave, onDelete, deleting }: { account: AzureAccount; pending: boolean; onSave: (i: AzureAccountInput) => void; onDelete: () => void; deleting: boolean }) {
  const [secret, setSecret] = useState('');
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="aws-form">
      <dl className="aws-facts">
        <dt>Tenant</dt>
        <dd className="mono">{account.tenant_name}</dd>
        <dt>Client ID</dt>
        <dd className="mono">{account.client_id}</dd>
        <dt>Client secret</dt>
        <dd>{account.has_secret ? 'Stored (encrypted)' : 'None'}</dd>
      </dl>
      <label className="rt-field">
        <span>New client secret</span>
        <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="new-password" placeholder="Replace the stored secret" />
      </label>
      <Save dirty={!!secret.trim()} pending={pending} onSave={() => onSave({ client_secret: secret.trim() })} />
      <div className="aws-danger">
        {confirm ? (
          <>
            <span>Remove this app registration? ObserveCraft stops collecting from its subscriptions; it stays in Azure.</span>
            <button className="btn" onClick={() => setConfirm(false)}>
              Cancel
            </button>
            <button className="btn btn-danger" disabled={deleting} onClick={onDelete}>
              {deleting ? 'Removing…' : 'Remove'}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => setConfirm(true)}>
            <Trash2 size={14} /> Remove app registration
          </button>
        )}
      </div>
    </div>
  );
}

function Metrics({ config, pending, onSave }: { config: AzureConfig; pending: boolean; onSave: (i: AzureAccountInput) => void }) {
  const [draft, setDraft] = useState(config);
  const set = <K extends keyof AzureConfig>(k: K, v: AzureConfig[K]) => setDraft((d) => ({ ...d, [k]: v }));
  const providerOn = (ns: string) => draft.resource_provider_configs.find((c) => c.namespace.toLowerCase() === ns.toLowerCase())?.metrics_enabled ?? draft.metrics_enabled_default;
  const setProvider = (ns: string, on: boolean) =>
    set('resource_provider_configs', [...draft.resource_provider_configs.filter((c) => c.namespace.toLowerCase() !== ns.toLowerCase()), { namespace: ns, metrics_enabled: on }]);
  return (
    <div className="aws-form">
      <Toggle label="Enable metric collection" value={draft.metrics_enabled} onChange={(v) => set('metrics_enabled', v)} />
      <Toggle
        label="Collect resource providers not listed below"
        hint="The default for resource providers without a setting."
        value={draft.metrics_enabled_default}
        onChange={(v) => set('metrics_enabled_default', v)}
      />
      <Toggle label="Mute monitors for expected VM shutdowns" value={draft.automute} onChange={(v) => set('automute', v)} />
      <Toggle label="Collect custom metrics" value={draft.custom_metrics_enabled} onChange={(v) => set('custom_metrics_enabled', v)} />
      <h4 className="aws-subhead">Resource providers</h4>
      <ul className="aws-source-list aws-namespaces">
        {PROVIDERS.map((ns) => (
          <li key={ns}>
            <span className="mono">{ns}</span>
            <button type="button" role="switch" aria-label={ns} aria-checked={providerOn(ns)} className={`switch${providerOn(ns) ? ' on' : ''}`} onClick={() => setProvider(ns, !providerOn(ns))} />
          </li>
        ))}
      </ul>
      <Save dirty={JSON.stringify(draft) !== JSON.stringify(config)} pending={pending} onSave={() => onSave(draft)} />
    </div>
  );
}

function Filters({ config, pending, onSave }: { config: AzureConfig; pending: boolean; onSave: (i: AzureAccountInput) => void }) {
  const [draft, setDraft] = useState({ host_filters: config.host_filters, app_service_plan_filters: config.app_service_plan_filters, container_app_filters: config.container_app_filters });
  const fields: [keyof typeof draft, string][] = [
    ['host_filters', 'Virtual machines'],
    ['app_service_plan_filters', 'App Service plans'],
    ['container_app_filters', 'Container apps'],
  ];
  return (
    <div className="aws-form">
      <div className="aws-note">
        <Info size={16} />
        <span>Comma-separated Azure tags (key:value, * wildcards); only resources with one of them are collected, and none with a tag preceded by !.</span>
      </div>
      {fields.map(([key, label]) => (
        <label key={key} className="rt-field">
          <span>{label}</span>
          <input value={draft[key]} onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))} placeholder="env:prod,!monitoring:off" />
        </label>
      ))}
      <Save dirty={fields.some(([k]) => draft[k] !== config[k])} pending={pending} onSave={() => onSave(draft)} />
    </div>
  );
}

function Resources({ config, pending, onSave }: { config: AzureConfig; pending: boolean; onSave: (i: AzureAccountInput) => void }) {
  const [draft, setDraft] = useState({ resource_collection_enabled: config.resource_collection_enabled, cspm_enabled: config.cspm_enabled });
  return (
    <div className="aws-form">
      <div className="aws-note">
        <Info size={16} />
        <span>Resource collection isn&apos;t built yet: these settings are stored for Terraform and API clients, and have no effect.</span>
      </div>
      <Toggle label="Resource collection" value={draft.resource_collection_enabled} onChange={(v) => setDraft((d) => ({ ...d, resource_collection_enabled: v }))} />
      <Toggle label="Cloud Security Management scanning" value={draft.cspm_enabled} onChange={(v) => setDraft((d) => ({ ...d, cspm_enabled: v }))} />
      <Save dirty={draft.resource_collection_enabled !== config.resource_collection_enabled || draft.cspm_enabled !== config.cspm_enabled} pending={pending} onSave={() => onSave(draft)} />
    </div>
  );
}

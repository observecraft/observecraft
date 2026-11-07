// Integrations → Amazon Web Services: the org's AWS accounts (the rows Terraform's
// `datadog_integration_aws_account` manages too) and the AWS services the integration collects.
import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Check, ChevronRight, Info, Plus, Search, Trash2, TriangleAlert } from 'lucide-react';
import type { AwsAccount, AwsAccountInput, AwsAccountStatus, AwsSetup } from '../../../api/aws/types';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { IntegrationLogo } from '../IntegrationLogo';
import { AWS_REGIONS } from './awsSetup';
import { LogSources } from './LogSources';
import { useAwsAccountMutations, useAwsAccounts, useAwsAccountStatuses, useAwsSetup } from './queries';
import { timeAgo } from '../../../lib/format';
import '../../settings/settings.css';
import '../integrations.css';
import './aws.css';

const DESCRIPTION = 'Amazon Web Services (AWS) is a collection of cloud services. The integration collects CloudWatch metrics from your AWS accounts and their logs through the Datadog Forwarder.';

export function AwsPage() {
  const [params, setParams] = useSearchParams();
  const tab = params.get('tab') === 'services' ? 'services' : 'configuration';
  const setup = useAwsSetup();
  const accounts = useAwsAccounts();
  const installed = (accounts.data?.length ?? 0) > 0;
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
        <span>Amazon Web Services</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo="amazonwebservices" name="Amazon Web Services" size={72} />
        <div>
          <h1>Amazon Web Services</h1>
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
          AWS Integrations
        </button>
      </nav>
      {setup.isPending || accounts.isPending ? (
        <Loading label="Loading AWS accounts…" />
      ) : setup.isError || accounts.isError ? (
        <ErrorState error={setup.error ?? accounts.error} onRetry={() => (setup.refetch(), accounts.refetch())} />
      ) : tab === 'services' ? (
        <Services setup={setup.data} />
      ) : accounts.data.length === 0 ? (
        <NoAccounts setup={setup.data} />
      ) : (
        <Accounts accounts={accounts.data} setup={setup.data} selected={params.get('account')} onSelect={(id) => setParam('account', id)} />
      )}
    </div>
  );
}

function NoAccounts({ setup }: { setup: AwsSetup }) {
  return (
    <div className="aws-empty">
      <p>Add an AWS account to collect its CloudWatch metrics and logs.</p>
      <Link className="btn btn-primary" to="/integrations/aws/add">
        <Plus size={15} /> Add AWS Account
      </Link>
      <Services setup={setup} />
    </div>
  );
}

function Services({ setup }: { setup: AwsSetup }) {
  return (
    <section className="aws-services">
      <h2>Includes {setup.namespaces.length} AWS integrations</h2>
      <div className="aws-service-grid">
        {setup.namespaces.map((n) => (
          <div key={n.namespace} className="aws-service" title={n.namespace}>
            <IntegrationLogo logo={`tile:${n.id}`} name={n.name} size={28} />
            <span>{n.name}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function Accounts({ accounts, setup, selected, onSelect }: { accounts: AwsAccount[]; setup: AwsSetup; selected: string | null; onSelect: (id: string | null) => void }) {
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  const shown = accounts.filter((a) => !q || a.attributes.aws_account_id.includes(q) || a.attributes.account_tags.some((t) => t.toLowerCase().includes(q)));
  const current = accounts.find((a) => a.id === selected) ?? shown[0] ?? accounts[0];
  const statuses = useAwsAccountStatuses(accounts.map((a) => a.id));
  const statusOf = (id: string) => statuses[accounts.findIndex((a) => a.id === id)]?.data;
  const hasIssues = (id: string) => !!statusOf(id)?.regions.some((r) => r.error && r.error.kind !== 'throttled');
  return (
    <>
      <div className="aws-toolbar">
        <label className="aws-search">
          <Search size={15} />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search AWS accounts by ID or tags" />
        </label>
        <Link className="btn btn-primary" to="/integrations/aws/add">
          <Plus size={15} /> Add AWS Account(s)
        </Link>
      </div>
      <div className="aws-split">
        <aside className="aws-account-list">
          <h3>Showing {shown.length} Accounts</h3>
          {shown.map((a) => (
            <button key={a.id} className={a.id === current.id ? 'selected' : ''} onClick={() => onSelect(a.id)}>
              <span className="aws-account-head">
                <strong>{a.attributes.aws_account_id}</strong>
                {hasIssues(a.id) && <span className="aws-warning-badge">WARNING</span>}
              </span>
              <span className="aws-tags">
                <span className="aws-tag">aws_account:{a.attributes.aws_account_id}</span>
                {a.attributes.account_tags.map((t) => (
                  <span key={t} className="aws-tag">
                    {t}
                  </span>
                ))}
              </span>
            </button>
          ))}
          {shown.length === 0 && <Empty>No account matches “{query}”.</Empty>}
        </aside>
        <AccountDetail key={current.id} account={current} status={statusOf(current.id)} setup={setup} onDeleted={() => onSelect(null)} />
      </div>
    </>
  );
}

const DETAIL_TABS = ['Issues', 'Account Details', 'General', 'Metric Collection', 'Log Collection', 'Resource Collection'] as const;
type DetailTab = (typeof DETAIL_TABS)[number];

function AccountDetail({ account, status, setup, onDeleted }: { account: AwsAccount; status: AwsAccountStatus | undefined; setup: AwsSetup; onDeleted: () => void }) {
  const [tab, setTab] = useState<DetailTab>('Issues');
  const { update, remove } = useAwsAccountMutations();
  const save = (input: AwsAccountInput) => update.mutate({ id: account.id, input });
  const a = account.attributes;
  return (
    <section className="aws-detail">
      <header>
        <h2>{a.aws_account_id}</h2>
      </header>
      <nav className="im-tabs">
        {DETAIL_TABS.map((t) => (
          <button key={t} className={t === tab ? 'active' : ''} onClick={() => setTab(t)}>
            {t}
            {t === 'Issues' && status?.regions.some((r) => r.error) && <span className="aws-dot" />}
          </button>
        ))}
      </nav>
      {update.error && <p className="form-error">{(update.error as Error).message}</p>}
      <div className="aws-detail-body">
        {tab === 'Issues' && <Issues status={status} metricsEnabled={a.metrics_config.enabled} />}
        {tab === 'Account Details' && (
          <AccountDetails account={account} pending={update.isPending} onSave={save} onDelete={() => remove.mutate(account.id, { onSuccess: onDeleted })} deleting={remove.isPending} />
        )}
        {tab === 'General' && <General account={account} pending={update.isPending} onSave={save} />}
        {tab === 'Metric Collection' && <Metrics account={account} setup={setup} pending={update.isPending} onSave={save} />}
        {tab === 'Log Collection' && <Logs account={account} setup={setup} pending={update.isPending} onSave={save} />}
        {tab === 'Resource Collection' && <Resources account={account} pending={update.isPending} onSave={save} />}
      </div>
    </section>
  );
}

const ISSUE_TITLES: Record<string, string> = {
  access_denied: 'ObserveCraft is not authorized to read some of your data.',
  auth: 'ObserveCraft cannot assume the integration role.',
  throttled: 'AWS is throttling ObserveCraft’s calls; collection retries with backoff.',
  disabled: 'The region is not enabled for this account.',
  other: 'Collection failed.',
};

const ISSUE_FIXES: Record<string, string> = {
  access_denied: 'to the integration role’s policy.',
  auth: 'Check that the role exists, trusts this install’s AWS principal, and requires the external ID shown in Account Details.',
  disabled: 'Enable the region in the AWS account, or remove it under General.',
};

function Issues({ status, metricsEnabled }: { status: AwsAccountStatus | undefined; metricsEnabled: boolean }) {
  if (!metricsEnabled)
    return (
      <div className="aws-note">
        <Info size={16} />
        <span>Metric collection is off for this account (Metric Collection).</span>
      </div>
    );
  if (!status) return <Loading label="Loading collection status…" />;
  const issues = status.regions.filter((r) => r.error);
  return (
    <div className="aws-form aws-issues">
      {issues.length === 0 ? (
        <div className="aws-note">
          <Check size={16} />
          <span>{status.regions.some((r) => r.last_success_at) ? 'No issues: every region is being collected.' : 'Collection starts within a few minutes of adding the account.'}</span>
        </div>
      ) : (
        issues.map((r) => (
          <div key={r.scope} className="aws-issue">
            <div>
              <span className="aws-warning-badge">{r.error!.kind === 'throttled' ? 'INFO' : 'WARNING'}</span> <strong>{ISSUE_TITLES[r.error!.kind] ?? ISSUE_TITLES.other}</strong>{' '}
              <span className="muted">
                {r.scope} · last seen {timeAgo(r.error!.at)}
              </span>
            </div>
            <p>
              {r.error!.missing_action ? (
                <>
                  Add <code>{r.error!.missing_action}</code> {ISSUE_FIXES.access_denied}
                </>
              ) : (
                (ISSUE_FIXES[r.error!.kind] ?? r.error!.message)
              )}
            </p>
            <small className="muted mono">{r.error!.message}</small>
          </div>
        ))
      )}
      <table className="im-table">
        <thead>
          <tr>
            <th>REGION</th>
            <th>COLLECTED UNTIL</th>
            <th>METRICS</th>
            <th>STATUS</th>
          </tr>
        </thead>
        <tbody>
          {status.regions.map((r) => (
            <tr key={r.scope}>
              <td className="mono">{r.scope}</td>
              <td>{r.collected_until ? timeAgo(r.collected_until) : '—'}</td>
              <td>{r.metrics}</td>
              <td>{r.error ? <TriangleAlert size={14} className="aws-warn-icon" /> : r.last_success_at ? <Check size={14} className="aws-ok-icon" /> : 'Pending'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface TabProps {
  account: AwsAccount;
  pending: boolean;
  onSave: (input: AwsAccountInput) => void;
}

function SaveBar({ dirty, pending, onSave }: { dirty: boolean; pending: boolean; onSave: () => void }) {
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

function AccountDetails({ account, pending, onSave, onDelete, deleting }: TabProps & { onDelete: () => void; deleting: boolean }) {
  const a = account.attributes;
  const [tags, setTags] = useState(a.account_tags.join(', '));
  const [confirm, setConfirm] = useState(false);
  const next = tags
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);
  const role = a.auth_config.role_name;
  return (
    <div className="aws-form">
      <dl className="aws-facts">
        <dt>AWS account ID</dt>
        <dd className="mono">{a.aws_account_id}</dd>
        <dt>Partition</dt>
        <dd>{a.aws_partition}</dd>
        <dt>Access</dt>
        <dd>{role ? `Role delegation: ${role}` : `Access key ${a.auth_config.access_key_id}`}</dd>
        {role && (
          <>
            <dt>External ID</dt>
            <dd className="mono">{a.auth_config.external_id}</dd>
          </>
        )}
        <dt>Added</dt>
        <dd>{new Date(a.created_at).toLocaleString()}</dd>
      </dl>
      <label className="rt-field">
        <span>Account tags</span>
        <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="key:value, key:value" />
        <span className="muted">Added to every metric and log from this account.</span>
      </label>
      <SaveBar dirty={next.join(',') !== a.account_tags.join(',')} pending={pending} onSave={() => onSave({ account_tags: next })} />
      <div className="aws-danger">
        {confirm ? (
          <>
            <span>Remove AWS account {a.aws_account_id}? ObserveCraft stops collecting from it; the IAM role stays in AWS.</span>
            <button className="btn" onClick={() => setConfirm(false)}>
              Cancel
            </button>
            <button className="btn btn-danger" disabled={deleting} onClick={onDelete}>
              {deleting ? 'Removing…' : 'Remove'}
            </button>
          </>
        ) : (
          <button className="btn" onClick={() => setConfirm(true)}>
            <Trash2 size={14} /> Remove account
          </button>
        )}
      </div>
    </div>
  );
}

function General({ account, pending, onSave }: TabProps) {
  const current = account.attributes.aws_regions;
  const [all, setAll] = useState(!!current.include_all);
  const [regions, setRegions] = useState<string[]>(current.include_only ?? []);
  const dirty = all !== !!current.include_all || (!all && regions.slice().sort().join() !== (current.include_only ?? []).slice().sort().join());
  return (
    <div className="aws-form">
      <h4 className="aws-subhead">Regions</h4>
      <Toggle label="Collect from all regions" hint="New regions are included automatically." value={all} onChange={setAll} />
      {!all && (
        <div className="aws-checks">
          {AWS_REGIONS.map((r) => (
            <label key={r}>
              <input type="checkbox" checked={regions.includes(r)} onChange={(e) => setRegions((rs) => (e.target.checked ? [...rs, r] : rs.filter((x) => x !== r)))} /> {r}
            </label>
          ))}
        </div>
      )}
      <SaveBar dirty={dirty && (all || regions.length > 0)} pending={pending} onSave={() => onSave({ aws_regions: all ? { include_all: true } : { include_only: regions } })} />
    </div>
  );
}

function Metrics({ account, setup, pending, onSave }: TabProps & { setup: AwsSetup }) {
  const m = account.attributes.metrics_config;
  const [draft, setDraft] = useState(m);
  const [query, setQuery] = useState('');
  // Namespaces are edited as the collected set; saved as exclude_only (or include_only when that's how they were set).
  const included = useMemo(() => {
    const all = setup.namespaces.map((n) => n.namespace);
    const f = draft.namespace_filters;
    return new Set(f.include_only ?? all.filter((n) => !(f.exclude_only ?? []).includes(n)));
  }, [draft.namespace_filters, setup.namespaces]);
  const setIncluded = (ns: string, on: boolean) => {
    const all = setup.namespaces.map((n) => n.namespace);
    const next = new Set(included);
    if (on) next.add(ns);
    else next.delete(ns);
    const filters = draft.namespace_filters.include_only ? { include_only: all.filter((n) => next.has(n)) } : { exclude_only: all.filter((n) => !next.has(n)) };
    setDraft((d) => ({ ...d, namespace_filters: filters }));
  };
  const q = query.trim().toLowerCase();
  const dirty = JSON.stringify(draft) !== JSON.stringify(m);
  return (
    <div className="aws-form">
      <Toggle label="Enable metric collection" value={draft.enabled} onChange={(enabled) => setDraft((d) => ({ ...d, enabled }))} />
      <Toggle label="Mute monitors for expected EC2 shutdowns" value={draft.automute_enabled} onChange={(automute_enabled) => setDraft((d) => ({ ...d, automute_enabled }))} />
      <Toggle
        label="Collect custom metrics"
        hint="Metrics in namespaces outside AWS's own."
        value={draft.collect_custom_metrics}
        onChange={(collect_custom_metrics) => setDraft((d) => ({ ...d, collect_custom_metrics }))}
      />
      <Toggle label="Collect CloudWatch alarms" value={draft.collect_cloudwatch_alarms} onChange={(collect_cloudwatch_alarms) => setDraft((d) => ({ ...d, collect_cloudwatch_alarms }))} />
      <h4 className="aws-subhead">Namespaces ({included.size} collected)</h4>
      <label className="aws-search">
        <Search size={15} />
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search namespaces" />
      </label>
      <ul className="aws-source-list aws-namespaces">
        {setup.namespaces
          .filter((n) => !q || n.name.toLowerCase().includes(q) || n.namespace.toLowerCase().includes(q))
          .map((n) => (
            <li key={n.namespace}>
              <span>
                {n.name} <span className="muted mono">{n.namespace}</span>
              </span>
              <button
                type="button"
                role="switch"
                aria-checked={included.has(n.namespace)}
                aria-label={n.name}
                className={`switch${included.has(n.namespace) ? ' on' : ''}`}
                onClick={() => setIncluded(n.namespace, !included.has(n.namespace))}
              />
            </li>
          ))}
      </ul>
      <SaveBar dirty={dirty} pending={pending} onSave={() => onSave({ metrics_config: draft })} />
    </div>
  );
}

function Logs({ account, setup, pending, onSave }: TabProps & { setup: AwsSetup }) {
  const f = account.attributes.logs_config.lambda_forwarder;
  const [lambdas, setLambdas] = useState(f.lambdas.join('\n'));
  const [sources, setSources] = useState(f.sources);
  const nextLambdas = lambdas
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
  const dirty = nextLambdas.join() !== f.lambdas.join() || sources.slice().sort().join() !== f.sources.slice().sort().join();
  return (
    <div className="aws-form">
      <label className="rt-field">
        <span>Forwarder Lambda ARNs</span>
        <textarea rows={3} value={lambdas} onChange={(e) => setLambdas(e.target.value)} placeholder="arn:aws:lambda:us-east-1:123456789012:function:observecraft-forwarder" />
        <span className="muted">One per line: the Datadog Forwarder functions deployed in this account, sending to ObserveCraft.</span>
      </label>
      <h4 className="aws-subhead">Log sources</h4>
      <LogSources services={setup.log_services} value={sources} onChange={setSources} />
      <SaveBar dirty={dirty} pending={pending} onSave={() => onSave({ logs_config: { lambda_forwarder: { lambdas: nextLambdas, sources, log_source_config: f.log_source_config } } })} />
    </div>
  );
}

function Resources({ account, pending, onSave }: TabProps) {
  const r = account.attributes.resources_config;
  const [draft, setDraft] = useState(r);
  return (
    <div className="aws-form">
      <div className="aws-note">
        <Info size={16} />
        <span>
          ObserveCraft scanning checks this account's configuration every day; its findings are in <Link to="/security/cloud/findings">Security → Cloud Security → Findings</Link>. The scans use
          short-lived read-only credentials: attach the AWS managed policies <code>SecurityAudit</code> and <code>job-function/ViewOnlyAccess</code> to the integration&apos;s role, plus the read-only
          actions <code>GET /api/v2/integration/aws/iam_permissions/resource_collection</code> lists. Extended resource collection isn&apos;t built yet (stored for Terraform and API clients only).
        </span>
      </div>
      <Toggle label="Extended resource collection" value={draft.extended_collection} onChange={(extended_collection) => setDraft((d) => ({ ...d, extended_collection }))} />
      <Toggle
        label="ObserveCraft scanning (Cloud Security)"
        value={draft.cloud_security_posture_management_collection}
        onChange={(cloud_security_posture_management_collection) => setDraft((d) => ({ ...d, cloud_security_posture_management_collection }))}
      />
      <SaveBar dirty={JSON.stringify(draft) !== JSON.stringify(r)} pending={pending} onSave={() => onSave({ resources_config: draft })} />
    </div>
  );
}

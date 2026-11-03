// Cloud Cost → Settings → Configure AWS / Azure: connect a provider's bill in numbered steps (the account
// or app registration whose credentials read it, where the bill files are), saved as Datadog's
// aws_cur_config / azure_uc_configs create attributes.
import { useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ChevronRight, CircleDollarSign } from 'lucide-react';
import type { AwsBillInput, AzureBillInput, AzureExportInput, BillInput, BillProvider } from '../../api/cloudCost/client';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { Step } from '../integrations/Step';
import { AWS_REGIONS } from '../integrations/aws/awsSetup';
import { useAwsAccounts } from '../integrations/aws/queries';
import { useAzureAccounts } from '../integrations/azure/queries';
import { azureScope, curBucketPolicy, validPrefix, type AzureScopeKind } from './billSetup';
import { useBillConfigMutations } from './billQueries';
import { PROVIDERS } from './ProvidersPage';
import '../integrations/integrations.css';
import '../integrations/aws/aws.css';
import './cloudCost.css';

interface StepDef {
  title: string;
  /** Why Next is disabled, or null when the step is complete. */
  missing: string | null;
  body: ReactNode;
}

/** Breadcrumbs, header, the numbered steps (one open at a time) and the Cancel / Previous / Next bar. */
function Wizard({ provider, crumb, steps, save }: { provider: BillProvider; crumb: string; steps: StepDef[]; save: Save }) {
  const [current, setCurrent] = useState(0);
  const p = PROVIDERS[provider];
  const last = current === steps.length - 1;
  const step = steps[current];
  const reachable = (i: number) => steps.slice(0, i).every((s) => !s.missing);
  return (
    <div className="ccm-editor ccm-connect">
      <header className="ccm-crumbs">
        <Link to="/cloud-cost">
          <CircleDollarSign size={18} /> Cloud Cost
        </Link>
        <ChevronRight size={16} className="muted-text" />
        <Link to="/cloud-cost/settings">Providers</Link>
        <ChevronRight size={16} className="muted-text" />
        <span>{crumb}</span>
      </header>
      <div className="ccm-editor-body ccm-connect-body">
        <h2 className="ccm-connect-title">
          <span className="ccm-connect-logos">
            <CircleDollarSign size={26} />
            <IntegrationLogo logo={p.logo} name={p.name} size={26} />
          </span>
          Configure {p.name} for Cloud Cost
        </h2>
        {steps.map((s, i) => (
          <Step key={s.title} n={i + 1} title={s.title} open={i === current} done={!s.missing && i < current} onToggle={() => reachable(i) && setCurrent(i)}>
            {s.body}
          </Step>
        ))}
        {save.error && <div className="ccm-form-error">{save.error.message}</div>}
      </div>
      <footer className="ccm-connect-footer">
        <Link className="btn" to="/cloud-cost/settings">
          Cancel
        </Link>
        <span className="ccm-connect-footer-right">
          {step.missing && <span className="muted-text">{step.missing}</span>}
          <button className="btn" disabled={current === 0} onClick={() => setCurrent((c) => c - 1)}>
            Previous
          </button>
          <button className="btn btn-primary" disabled={!!step.missing || save.pending} onClick={() => (last ? save.run() : setCurrent((c) => c + 1))}>
            {last ? (save.pending ? 'Saving…' : 'Save') : 'Next'}
          </button>
        </span>
      </footer>
    </div>
  );
}

interface Save {
  run: () => void;
  pending: boolean;
  error: Error | null;
}

/** Creates the config, then goes back to the provider list; `attributes` is read when Save is pressed. */
function useSave<P extends BillProvider>(provider: P, attributes: () => BillInput<P> | null): Save {
  const navigate = useNavigate();
  const { create } = useBillConfigMutations();
  return {
    run: () => {
      const a = attributes();
      if (a) create.mutate({ provider, attributes: a }, { onSuccess: () => navigate('/cloud-cost/settings') });
    },
    pending: create.isPending,
    error: create.error as Error | null,
  };
}

function Field({ label, hint, error, children }: { label: string; hint?: ReactNode; error?: string | null; children: ReactNode }) {
  return (
    <label className="rt-field">
      <span>
        {label} <span className="rt-required">*</span>
      </span>
      {children}
      {error ? <span className="form-error">{error}</span> : hint ? <span className="muted-text">{hint}</span> : null}
    </label>
  );
}

/* ---------------------------------------------------------------- AWS */

export function ConnectAwsBillPage() {
  const accounts = useAwsAccounts();
  const [account, setAccount] = useState('');
  const [bill, setBill] = useState<Omit<AwsBillInput, 'account_id'>>({ bucket_name: '', bucket_region: 'us-east-1', report_prefix: '', report_name: '' });
  const save = useSave('aws', () => ({ account_id: account, ...bill, report_prefix: bill.report_prefix.trim() }));
  const set = (patch: Partial<typeof bill>) => setBill((b) => ({ ...b, ...patch }));
  if (accounts.isPending) return <Loading label="Loading AWS accounts…" />;
  if (accounts.isError) return <ErrorState error={accounts.error} onRetry={() => accounts.refetch()} />;
  const ids = accounts.data.map((a) => a.attributes.aws_account_id);
  const prefixOk = validPrefix(bill.report_prefix.trim());

  const steps: StepDef[] = [
    {
      title: 'Select AWS account',
      missing: account ? null : 'Select an account.',
      body: ids.length ? (
        <>
          <p>The management (payer) account the Cost and Usage Report is exported from. ObserveCraft reads it with that account&apos;s integration role.</p>
          <Field label="AWS account">
            <select value={account} onChange={(e) => setAccount(e.target.value)}>
              <option value="">Select an account</option>
              {ids.map((id) => (
                <option key={id}>{id}</option>
              ))}
            </select>
          </Field>
        </>
      ) : (
        <p>
          No AWS account is connected yet. Add the payer account in <Link to="/integrations/aws/add">Integrations → Amazon Web Services</Link> first, then come back here.
        </p>
      ),
    },
    {
      title: 'Cost and Usage Report',
      missing: !bill.bucket_name.trim() || !bill.report_prefix.trim() || !bill.report_name.trim() ? 'Enter the bucket, prefix and export name.' : !prefixOk ? 'Fix the export path prefix.' : null,
      body: (
        <>
          <p>
            In the management (payer) account, open Billing and Cost Management → Data exports and create a <strong>standard data export</strong> of <strong>CUR 2.0</strong>. Include resource IDs,
            choose Parquet with &quot;Overwrite existing data export file&quot;, and deliver it to an S3 bucket. AWS writes the first files within a day.
          </p>
          <p>
            A classic Cost and Usage Report (Billing and Cost Management → Cost and usage reports, gzip CSV or Parquet, with resource IDs) works too: its S3 path prefix and report name go in the same
            fields.
          </p>
          <p>Then let the integration role read the bucket: add this statement to its permissions (it lists the bucket and reads its objects).</p>
          <CodeBlock code={JSON.stringify(curBucketPolicy(bill.bucket_name), null, 2)} filename="S3 read policy" />
          <div className="ccm-connect-fields">
            <Field label="Bucket name">
              <input value={bill.bucket_name} onChange={(e) => set({ bucket_name: e.target.value.trim() })} placeholder="example-org-cur" />
            </Field>
            <Field label="Bucket region">
              <select value={bill.bucket_region} onChange={(e) => set({ bucket_region: e.target.value })}>
                {AWS_REGIONS.map((r) => (
                  <option key={r}>{r}</option>
                ))}
              </select>
            </Field>
            <Field label="Export path prefix" hint="The S3 path prefix of the export, without leading or trailing '/'." error={prefixOk ? null : "The prefix can't start or end with '/'."}>
              <input value={bill.report_prefix} onChange={(e) => set({ report_prefix: e.target.value })} placeholder="cur" />
            </Field>
            <Field label="Export name">
              <input value={bill.report_name} onChange={(e) => set({ report_name: e.target.value.trim() })} placeholder="observecraft-costs" />
            </Field>
          </div>
        </>
      ),
    },
  ];
  return <Wizard provider="aws" crumb="Configure AWS Account" steps={steps} save={save} />;
}

/* ---------------------------------------------------------------- Azure */

const emptyExport = (): AzureExportInput => ({ storage_account: '', storage_container: '', export_path: '', export_name: '' });
const exportReady = (e: AzureExportInput) => Object.values(e).every((v) => v.trim());

function ExportFields({ value, onChange, location = true }: { value: AzureExportInput; onChange: (v: AzureExportInput) => void; location?: boolean }) {
  const set = (patch: Partial<AzureExportInput>) => onChange({ ...value, ...patch });
  return (
    <div className="ccm-connect-fields">
      {location && (
        <>
          <Field label="Storage account">
            <input value={value.storage_account} onChange={(e) => set({ storage_account: e.target.value.trim() })} placeholder="exampleorgcosts" />
          </Field>
          <Field label="Container">
            <input value={value.storage_container} onChange={(e) => set({ storage_container: e.target.value.trim() })} placeholder="billing" />
          </Field>
          <Field label="Directory">
            <input value={value.export_path} onChange={(e) => set({ export_path: e.target.value.trim() })} placeholder="exports" />
          </Field>
        </>
      )}
      <Field label="Export name">
        <input value={value.export_name} onChange={(e) => set({ export_name: e.target.value.trim() })} />
      </Field>
    </div>
  );
}

export function ConnectAzureBillPage() {
  const registrations = useAzureAccounts();
  const [registration, setRegistration] = useState('');
  const [scopeKind, setScopeKind] = useState<AzureScopeKind>('subscription');
  const [scopeId, setScopeId] = useState('');
  const [same, setSame] = useState(true);
  const [actual, setActual] = useState<AzureExportInput>({ ...emptyExport(), export_name: 'observecraft-actual' });
  const [amortized, setAmortized] = useState<AzureExportInput>({ ...emptyExport(), export_name: 'observecraft-amortized' });
  const reg = registrations.data?.find((r) => r.id === registration);
  const amortizedFull = same ? { ...amortized, storage_account: actual.storage_account, storage_container: actual.storage_container, export_path: actual.export_path } : amortized;
  const save = useSave('azure', (): AzureBillInput | null =>
    reg ? { account_id: reg.tenant_name, client_id: reg.client_id, scope: azureScope(scopeKind, scopeId), actual_bill_config: actual, amortized_bill_config: amortizedFull } : null,
  );
  if (registrations.isPending) return <Loading label="Loading app registrations…" />;
  if (registrations.isError) return <ErrorState error={registrations.error} onRetry={() => registrations.refetch()} />;
  const regs = registrations.data;

  const steps: StepDef[] = [
    {
      title: 'Select app registration',
      missing: reg ? null : 'Select an app registration.',
      body: regs.length ? (
        <>
          <p>ObserveCraft reads the exports with the Azure integration&apos;s app registration.</p>
          <Field label="App registration">
            <select value={registration} onChange={(e) => setRegistration(e.target.value)}>
              <option value="">Select an app registration</option>
              {regs.map((r) => (
                <option key={r.id} value={r.id}>
                  Tenant {r.tenant_name} · client {r.client_id}
                </option>
              ))}
            </select>
          </Field>
        </>
      ) : (
        <p>
          No app registration is connected yet. Add one in <Link to="/integrations/azure/add">Integrations → Microsoft Azure</Link> first, then come back here.
        </p>
      ),
    },
    {
      title: 'Scope',
      missing: scopeId.trim() ? null : 'Enter the scope ID.',
      body: (
        <>
          <p>The scope the cost exports cover: a subscription, or a whole billing account.</p>
          <div className="aws-segments ccm-connect-segments" role="tablist">
            <button type="button" className={scopeKind === 'subscription' ? 'active' : ''} onClick={() => setScopeKind('subscription')}>
              Subscription
            </button>
            <button type="button" className={scopeKind === 'billing-account' ? 'active' : ''} onClick={() => setScopeKind('billing-account')}>
              Billing account
            </button>
          </div>
          <Field label={scopeKind === 'subscription' ? 'Subscription ID' : 'Billing account ID'} hint={scopeId.trim() ? <span className="mono">{azureScope(scopeKind, scopeId)}</span> : undefined}>
            <input value={scopeId} onChange={(e) => setScopeId(e.target.value.trim())} />
          </Field>
        </>
      ),
    },
    {
      title: 'Cost exports',
      missing: exportReady(actual) && exportReady(amortizedFull) ? null : 'Fill in both exports.',
      body: (
        <>
          <p>
            In the Azure portal, open Cost Management → Exports for this scope and create two <strong>daily</strong> exports of <strong>Cost and usage details</strong>: one of actual costs and one of
            amortized costs. Turn file partitioning on and overwriting off, choose CSV or Parquet, and write them to a container of a storage account.
          </p>
          <p>
            Then grant the app registration the <strong>Storage Blob Data Reader</strong> role on that storage account (Access control (IAM) → Add role assignment).
          </p>
          <label className="ccm-check">
            <input type="checkbox" checked={same} onChange={(e) => setSame(e.target.checked)} /> Both exports use the same storage account and directory
          </label>
          <h4>Actual costs export</h4>
          <ExportFields value={actual} onChange={setActual} />
          <h4>Amortized costs export</h4>
          <ExportFields value={amortized} onChange={setAmortized} location={!same} />
        </>
      ),
    },
  ];
  return <Wizard provider="azure" crumb="Configure Azure Account" steps={steps} save={save} />;
}

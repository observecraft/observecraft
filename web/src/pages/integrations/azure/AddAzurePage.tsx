// Integrations → Microsoft Azure → Add New App Registration: Terraform (the app registration, its role
// assignments and the integration through Datadog's provider) or an existing app registration (created
// with the Azure CLI command shown), which ObserveCraft signs in with.
import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Check, ChevronRight } from 'lucide-react';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { IntegrationLogo } from '../IntegrationLogo';
import { Step } from '../Step';
import { cliCommand, terraformConfig, validGuid } from './azureSetup';
import { useAzureAccountMutations } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';

type Method = 'existing' | 'terraform';

const METHODS: { id: Method; title: string; text: string; recommended?: boolean }[] = [
  { id: 'existing', title: 'Existing App Registration', text: 'Create a service principal with the Azure CLI (or use one you have) and enter its credentials.', recommended: true },
  { id: 'terraform', title: 'Terraform', text: 'Create the app registration, its role assignments and the integration with Terraform.' },
];

export function AddAzurePage() {
  const navigate = useNavigate();
  const { create } = useAzureAccountMutations();
  const [method, setMethod] = useState<Method>('existing');
  const [subscriptions, setSubscriptions] = useState('');
  const [hostFilters, setHostFilters] = useState('');
  const [tenant, setTenant] = useState('');
  const [client, setClient] = useState('');
  const [secret, setSecret] = useState('');
  const subs = subscriptions
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const subsOk = subs.every(validGuid);
  const ready = validGuid(tenant) && validGuid(client) && secret.trim().length > 0;
  const submit = () =>
    create.mutate(
      { tenant_name: tenant.trim(), client_id: client.trim(), client_secret: secret.trim(), host_filters: hostFilters.trim() },
      { onSuccess: (a) => navigate(`/integrations/azure?account=${a.id}`) },
    );

  return (
    <div className="rt-create-page aws-add-page">
      <header className="product-header">
        <div className="product-title rt-crumb">
          <Link to="/integrations">Integrations</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <Link to="/integrations/azure">Microsoft Azure</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <span>Add New App Registration</span>
        </div>
      </header>
      <div className="rt-create aws-add">
        <h3 className="aws-title">
          <IntegrationLogo logo="tile:azure" name="Microsoft Azure" size={26} /> Add New App Registration
        </h3>
        <h4 className="aws-subhead">Select setup method</h4>
        <div className="aws-methods">
          {METHODS.map((m) => (
            <button key={m.id} type="button" className={`aws-method${method === m.id ? ' selected' : ''}`} onClick={() => setMethod(m.id)}>
              <strong>
                {method === m.id && <Check size={15} />} {m.title} {m.recommended && <span className="aws-recommended">RECOMMENDED</span>}
              </strong>
              <span>{m.text}</span>
            </button>
          ))}
        </div>

        <Step n={1} title="General Configuration" open onToggle={() => {}} done={subs.length > 0 && subsOk}>
          <label className="rt-field">
            <span>Subscriptions</span>
            <input value={subscriptions} onChange={(e) => setSubscriptions(e.target.value)} placeholder="Subscription IDs, comma-separated" />
            {!subsOk && <span className="form-error">Subscription IDs are GUIDs.</span>}
            <span className="muted">The subscriptions the app registration gets Monitoring Reader on; ObserveCraft collects every subscription it can read.</span>
          </label>
          <label className="rt-field">
            <span>Virtual machine filters</span>
            <input value={hostFilters} onChange={(e) => setHostFilters(e.target.value)} placeholder="env:prod (optional)" />
          </label>
        </Step>

        {method === 'existing' ? (
          <>
            <Step n={2} title="Create the service principal" open onToggle={() => {}}>
              <p>
                In Azure Cloud Shell (bash), create an app registration with the Monitoring Reader role on the subscriptions. Its output&apos;s <code>tenant</code>, <code>appId</code> and{' '}
                <code>password</code> are the tenant, client ID and client secret below.
              </p>
              <CodeBlock code={cliCommand(subs)} filename="Azure CLI" />
            </Step>
            <Step n={3} title="Enter the app registration" open onToggle={() => {}} done={ready}>
              <label className="rt-field">
                <span>Tenant ID</span>
                <input value={tenant} onChange={(e) => setTenant(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" />
              </label>
              <label className="rt-field">
                <span>Client ID</span>
                <input value={client} onChange={(e) => setClient(e.target.value)} placeholder="00000000-0000-0000-0000-000000000000" />
              </label>
              <label className="rt-field">
                <span>Client secret</span>
                <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="new-password" />
                <span className="muted">Stored encrypted and never shown again. Renew it here before it expires.</span>
              </label>
              {create.error ? <p className="form-error">{(create.error as Error).message}</p> : null}
              <div className="rt-create-actions">
                <Link className="btn" to="/integrations/azure">
                  Cancel
                </Link>
                <button type="button" className="btn btn-primary" disabled={!ready || create.isPending} onClick={submit}>
                  {create.isPending ? 'Adding…' : 'Add app registration'}
                </button>
              </div>
            </Step>
          </>
        ) : (
          <>
            <Step n={2} title="Copy Terraform HCL" open onToggle={() => {}}>
              <p>
                Set <code>DD_API_KEY</code> to an ObserveCraft ingest key and <code>DD_APP_KEY</code> to an application key, and sign in to Azure (<code>az login</code>) before applying.
              </p>
              <CodeBlock code={terraformConfig({ subscriptions: subs, apiUrl: window.location.origin, hostFilters: hostFilters.trim() })} filename="observecraft-azure.tf" />
            </Step>
            <Step n={3} title="Apply Terraform configuration" open onToggle={() => {}}>
              <p>Run &quot;terraform apply&quot;: the app registration appears in the list once it is created.</p>
              <div className="rt-create-actions">
                <Link className="btn btn-primary" to="/integrations/azure">
                  Go to app registrations
                </Link>
              </div>
            </Step>
          </>
        )}
      </div>
    </div>
  );
}

// Integrations → Amazon Web Services → Add New AWS Account: CloudFormation (our template creating the
// integration role), Terraform (the role and the account through Datadog's provider) or manually (a role
// created in the console, or access keys), plus the Datadog Forwarder for logs.
import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Check, ChevronRight, Download, ExternalLink, TriangleAlert } from 'lucide-react';
import type { AwsAccountInput, AwsSetup } from '../../../api/aws/types';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { IntegrationLogo } from '../IntegrationLogo';
import { Step } from '../Step';
import { AWS_REGIONS, cloudFormationTemplate, DEFAULT_ROLE_NAME, forwarderStackUrl, permissionsPolicy, terraformConfig, trustPolicy, validAccountId, validRoleName } from './awsSetup';
import { LogSources } from './LogSources';
import { useAwsAccountMutations, useAwsSetup, useExternalId } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import './aws.css';

type Method = 'cloudformation' | 'terraform' | 'manual';

const METHODS: { id: Method; title: string; text: string; recommended?: boolean }[] = [
  { id: 'cloudformation', title: 'CloudFormation', text: 'Create the integration role with a CloudFormation stack, then add the account here.', recommended: true },
  { id: 'terraform', title: 'Terraform', text: 'Create the IAM role and the ObserveCraft account with Terraform.' },
  { id: 'manual', title: 'Manually', text: 'Create the role in the AWS console, or use access keys, then finish here.' },
];

/** Where Datadog's clients reach this install: the API (the page's origin) and the intake (port 4318). */
const apiUrl = () => window.location.origin;
const intakeHost = () => window.location.hostname;
const INTAKE_PORT = '4318';

function download(filename: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const a = Object.assign(document.createElement('a'), { href: url, download: filename });
  a.click();
  URL.revokeObjectURL(url);
}

export function AddAwsAccountPage() {
  const setup = useAwsSetup();
  return (
    <div className="rt-create-page aws-add-page">
      <header className="product-header">
        <div className="product-title rt-crumb">
          <Link to="/integrations">Integrations</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <Link to="/integrations/aws">Amazon Web Services</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <span>Add New AWS Account</span>
        </div>
      </header>
      {setup.isPending ? <Loading label="Loading…" /> : setup.isError ? <ErrorState error={setup.error} onRetry={() => setup.refetch()} /> : <AddAccount setup={setup.data} />}
    </div>
  );
}

function AddAccount({ setup }: { setup: AwsSetup }) {
  const navigate = useNavigate();
  const { create } = useAwsAccountMutations();
  const externalId = useExternalId();
  const [method, setMethod] = useState<Method>('cloudformation');
  const [accountId, setAccountId] = useState('');
  const [roleName, setRoleName] = useState(DEFAULT_ROLE_NAME);
  const [auth, setAuth] = useState<'role' | 'keys'>('role');
  const [keyId, setKeyId] = useState('');
  const [secret, setSecret] = useState('');
  const [forwarder, setForwarder] = useState(true);
  const [region, setRegion] = useState('us-east-1');
  const [sources, setSources] = useState<string[]>([]);
  const principal = setup.principal_arn;
  const accountOk = validAccountId(accountId);
  const roleOk = validRoleName(roleName);
  const usesRole = method !== 'manual' || auth === 'role';
  const ready = accountOk && (usesRole ? roleOk && !!externalId.data && !!principal : keyId.trim() && secret.trim());

  const submit = () => {
    const input: AwsAccountInput = {
      aws_account_id: accountId,
      auth_config: usesRole ? { role_name: roleName, external_id: externalId.data } : { access_key_id: keyId.trim(), secret_access_key: secret.trim() },
      logs_config: { lambda_forwarder: { lambdas: [], sources, log_source_config: { tag_filters: [] } } },
    };
    create.mutate(input, { onSuccess: (a) => navigate(`/integrations/aws?account=${a.id}`) });
  };

  const role = principal && externalId.data ? { principalArn: principal, externalId: externalId.data, roleName, permissions: setup.permissions } : null;
  const accountStep = (
    <Step n={1} title="Enter Account ID" open onToggle={() => {}} done={accountOk}>
      <label className="rt-field">
        <span>
          Account ID <span className="rt-required">*</span>
        </span>
        <input value={accountId} onChange={(e) => setAccountId(e.target.value.trim())} placeholder="AWS Account ID" inputMode="numeric" maxLength={12} autoFocus />
        {accountId && !accountOk && <span className="form-error">An AWS account ID has 12 digits.</span>}
      </label>
      {usesRole && (
        <label className="rt-field">
          <span>IAM role name</span>
          <input value={roleName} onChange={(e) => setRoleName(e.target.value.trim())} maxLength={64} />
          {!roleOk && <span className="form-error">Up to 64 letters, digits and +=,.@_- characters.</span>}
        </label>
      )}
    </Step>
  );

  const logsStep = (n: number) => (
    <Step n={n} title="Send AWS Logs to ObserveCraft" open onToggle={() => {}} done={sources.length > 0}>
      <p>
        The Datadog Forwarder (a Lambda function, Apache-2.0) sends logs from CloudWatch Logs and S3 to ObserveCraft&apos;s intake. Deploy it in each region you collect logs from; enter an
        ObserveCraft ingest key as its <code>DdApiKey</code> in the console.
      </p>
      <label className="aws-inline">
        <button type="button" role="switch" aria-checked={forwarder} className={`switch${forwarder ? ' on' : ''}`} onClick={() => setForwarder((f) => !f)} />
        Deploy the log forwarder
      </label>
      {forwarder && (
        <div className="aws-forwarder">
          <label className="rt-field aws-region">
            <span>Region</span>
            <select value={region} onChange={(e) => setRegion(e.target.value)}>
              {AWS_REGIONS.map((r) => (
                <option key={r}>{r}</option>
              ))}
            </select>
          </label>
          <a className="btn" href={forwarderStackUrl({ region, intakeHost: intakeHost(), intakePort: INTAKE_PORT, apiUrl: apiUrl() })} target="_blank" rel="noreferrer">
            Launch the forwarder stack <ExternalLink size={14} />
          </a>
        </div>
      )}
      <h4 className="aws-subhead">Log sources</h4>
      <p className="muted">The services whose logs the forwarder subscribes to.</p>
      <LogSources services={setup.log_services} value={sources} onChange={setSources} />
    </Step>
  );

  return (
    <div className="rt-create aws-add">
      <h3 className="aws-title">
        <IntegrationLogo logo="amazonwebservices" name="Amazon Web Services" size={26} /> Add New AWS Account
      </h3>
      {!principal && (
        <div className="rt-warning">
          <TriangleAlert size={16} />
          <span>
            This install has no AWS identity to assume integration roles with. Set <code>OC_AWS_PRINCIPAL_ARN</code> on the server to the IAM role it runs as; until then only access keys work
            (Manually → Access keys).
          </span>
        </div>
      )}
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

      {method === 'cloudformation' && (
        <>
          {accountStep}
          {logsStep(2)}
          <Step n={3} title="Create the integration role" open onToggle={() => {}}>
            {role ? (
              <>
                <p>
                  Create a stack from this template in the account (CloudFormation → Create stack → Upload a template file), or with the AWS CLI. It creates the role ObserveCraft assumes, trusting
                  this install only with the external ID below.
                </p>
                <CodeBlock code={cloudFormationTemplate(role)} filename="observecraft-aws-integration.json" />
                <div className="aws-actions-row">
                  <button type="button" className="btn" onClick={() => download('observecraft-aws-integration.json', cloudFormationTemplate(role))}>
                    <Download size={14} /> Download template
                  </button>
                </div>
                <CodeBlock
                  code={`aws cloudformation deploy --stack-name observecraft-integration --template-file observecraft-aws-integration.json --capabilities CAPABILITY_NAMED_IAM`}
                  filename="AWS CLI"
                />
              </>
            ) : (
              <p className="muted">{principal ? 'Generating an external ID…' : 'Needs OC_AWS_PRINCIPAL_ARN on the server.'}</p>
            )}
          </Step>
          <Step n={4} title="Add the account" open onToggle={() => {}}>
            <p>Once the stack is created, add the account. ObserveCraft starts with all regions and the default namespaces; change them on the account afterwards.</p>
            <Submit ready={!!ready} pending={create.isPending} error={create.error} onSubmit={submit} />
          </Step>
        </>
      )}

      {method === 'terraform' && (
        <>
          {accountStep}
          {logsStep(2)}
          <Step n={3} title="Copy Terraform HCL" open onToggle={() => {}}>
            {principal ? (
              <>
                <p>
                  Set up your Terraform configuration with the template below. Set <code>DD_API_KEY</code> to an ObserveCraft ingest key and <code>DD_APP_KEY</code> to an application key.
                </p>
                <CodeBlock
                  code={terraformConfig({
                    principalArn: principal,
                    roleName,
                    accountId: accountId || '<AWS_ACCOUNT_ID>',
                    permissions: setup.permissions,
                    apiUrl: apiUrl(),
                    regions: [],
                    logSources: sources,
                  })}
                  filename="observecraft-aws.tf"
                />
              </>
            ) : (
              <p className="muted">Needs OC_AWS_PRINCIPAL_ARN on the server.</p>
            )}
          </Step>
          <Step n={4} title="Apply Terraform configuration" open onToggle={() => {}}>
            <p>Run &quot;terraform apply&quot; to create the IAM role and policy in AWS and the account in ObserveCraft; it then appears in the account list.</p>
            <div className="rt-create-actions">
              <Link className="btn btn-primary" to="/integrations/aws">
                Go to AWS accounts
              </Link>
            </div>
          </Step>
        </>
      )}

      {method === 'manual' && (
        <>
          {accountStep}
          <Step n={2} title="Grant ObserveCraft access" open onToggle={() => {}}>
            <div className="aws-segments" role="tablist">
              <button type="button" className={auth === 'role' ? 'active' : ''} onClick={() => setAuth('role')}>
                Role delegation
              </button>
              <button type="button" className={auth === 'keys' ? 'active' : ''} onClick={() => setAuth('keys')}>
                Access keys
              </button>
            </div>
            {auth === 'role' ? (
              role ? (
                <>
                  <p>
                    In IAM, create the role <strong>{roleName}</strong> with this trust policy (Custom trust policy), then attach an inline policy with the permissions below.
                  </p>
                  <CodeBlock code={JSON.stringify(trustPolicy(role.principalArn, role.externalId), null, 2)} filename="Trust policy" />
                  <CodeBlock code={JSON.stringify(permissionsPolicy(setup.permissions), null, 2)} filename="Permissions policy" />
                </>
              ) : (
                <p className="muted">{principal ? 'Generating an external ID…' : 'Role delegation needs OC_AWS_PRINCIPAL_ARN on the server.'}</p>
              )
            ) : (
              <>
                <p>Create an IAM user with the permissions policy below and an access key. ObserveCraft stores the secret encrypted and never shows it again.</p>
                <CodeBlock code={JSON.stringify(permissionsPolicy(setup.permissions), null, 2)} filename="Permissions policy" />
                <label className="rt-field">
                  <span>Access key ID</span>
                  <input value={keyId} onChange={(e) => setKeyId(e.target.value)} placeholder="AKIA…" autoComplete="off" />
                </label>
                <label className="rt-field">
                  <span>Secret access key</span>
                  <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="new-password" />
                </label>
              </>
            )}
          </Step>
          {logsStep(3)}
          <Step n={4} title="Add the account" open onToggle={() => {}}>
            <Submit ready={!!ready} pending={create.isPending} error={create.error} onSubmit={submit} />
          </Step>
        </>
      )}
    </div>
  );
}

function Submit({ ready, pending, error, onSubmit }: { ready: boolean; pending: boolean; error: unknown; onSubmit: () => void }) {
  return (
    <>
      {error ? <p className="form-error">{(error as Error).message}</p> : null}
      <div className="rt-create-actions">
        <Link className="btn" to="/integrations/aws">
          Cancel
        </Link>
        <button type="button" className="btn btn-primary" disabled={!ready || pending} onClick={onSubmit}>
          {pending ? 'Adding…' : 'Add account'}
        </button>
      </div>
    </>
  );
}

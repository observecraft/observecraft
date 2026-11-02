// Integrations → Google Cloud → Add New GCP Account: gcloud commands or Terraform create a service
// account with read roles on the projects and let this install impersonate it; then it is added here.
import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Check, ChevronRight, TriangleAlert } from 'lucide-react';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { IntegrationLogo } from '../IntegrationLogo';
import { Step } from '../Step';
import { gcloudCommands, serviceAccountEmail, terraformConfig, validProjectId } from './gcpSetup';
import { useGcpAccountMutations, useGcpSetup } from './queries';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';

type Method = 'gcloud' | 'terraform';

const METHODS: { id: Method; title: string; text: string; recommended?: boolean }[] = [
  { id: 'gcloud', title: 'Cloud Shell', text: 'Run the gcloud commands in Google Cloud Shell, then add the service account here.', recommended: true },
  { id: 'terraform', title: 'Terraform', text: 'Create the service account, its roles and the integration with Terraform.' },
];

export function AddGcpPage() {
  const setup = useGcpSetup();
  const navigate = useNavigate();
  const { create } = useGcpAccountMutations();
  const [method, setMethod] = useState<Method>('gcloud');
  const [projects, setProjects] = useState('');
  const list = projects
    .split(/[\s,]+/)
    .map((p) => p.trim())
    .filter(Boolean);
  const valid = list.length > 0 && list.every(validProjectId);
  const [email, setEmail] = useState('');
  const shownEmail = email || (list[0] ? serviceAccountEmail(list[0]) : '');
  const principal = setup.data?.principal_email ?? null;
  return (
    <div className="rt-create-page aws-add-page">
      <header className="product-header">
        <div className="product-title rt-crumb">
          <Link to="/integrations">Integrations</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <Link to="/integrations/gcp">Google Cloud Platform</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <span>Add New GCP Account</span>
        </div>
      </header>
      {setup.isPending ? (
        <Loading label="Loading…" />
      ) : setup.isError ? (
        <ErrorState error={setup.error} onRetry={() => setup.refetch()} />
      ) : (
        <div className="rt-create aws-add">
          <h3 className="aws-title">
            <IntegrationLogo logo="googlecloud" name="Google Cloud Platform" size={26} /> Add New GCP Account
          </h3>
          {!principal && (
            <div className="rt-warning">
              <TriangleAlert size={16} />
              <span>
                This install has no Google service account to impersonate yours with. Set <code>OC_GCP_PRINCIPAL_EMAIL</code> on the server to the service account it runs as (workload identity or a
                key in <code>GOOGLE_APPLICATION_CREDENTIALS</code>).
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
          <Step n={1} title="Provide GCP Resources" open onToggle={() => {}} done={valid}>
            <label className="rt-field">
              <span>
                Projects <span className="rt-required">*</span>
              </span>
              <input value={projects} onChange={(e) => setProjects(e.target.value)} placeholder="Project IDs, comma-separated" />
              {list.length > 0 && !valid && <span className="form-error">Project IDs have 6 to 30 lowercase letters, digits and hyphens.</span>}
              <span className="muted">The service account is created in the first one and gets read roles on all of them.</span>
            </label>
          </Step>
          <Step n={2} title={method === 'gcloud' ? 'Create the service account' : 'Copy Terraform HCL'} open onToggle={() => {}}>
            {principal ? (
              method === 'gcloud' ? (
                <>
                  <p>Run these commands in Google Cloud Shell. They create the service account, grant it Monitoring Viewer and Browser on the projects, and let this install impersonate it.</p>
                  <CodeBlock code={gcloudCommands({ projects: list, principal })} filename="Cloud Shell" />
                </>
              ) : (
                <>
                  <p>
                    Set <code>DD_API_KEY</code> to an ObserveCraft ingest key and <code>DD_APP_KEY</code> to an application key; apply, and the service account appears in the list.
                  </p>
                  <CodeBlock code={terraformConfig({ projects: list, principal, apiUrl: window.location.origin })} filename="observecraft-gcp.tf" />
                </>
              )
            ) : (
              <p className="muted">Needs OC_GCP_PRINCIPAL_EMAIL on the server.</p>
            )}
          </Step>
          {method === 'gcloud' && (
            <Step n={3} title="Add the service account" open onToggle={() => {}}>
              <label className="rt-field">
                <span>Service account email</span>
                <input value={shownEmail} onChange={(e) => setEmail(e.target.value)} placeholder="observecraft-integration@<project>.iam.gserviceaccount.com" />
              </label>
              {create.error ? <p className="form-error">{(create.error as Error).message}</p> : null}
              <div className="rt-create-actions">
                <Link className="btn" to="/integrations/gcp">
                  Cancel
                </Link>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={!shownEmail.includes('@') || create.isPending}
                  onClick={() => create.mutate({ client_email: shownEmail.trim() }, { onSuccess: (a) => navigate(`/integrations/gcp?account=${a.id}`) })}
                >
                  {create.isPending ? 'Adding…' : 'Add service account'}
                </button>
              </div>
            </Step>
          )}
        </div>
      )}
    </div>
  );
}

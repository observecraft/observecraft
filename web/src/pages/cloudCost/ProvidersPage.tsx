// Cloud Cost → Settings → Providers: the providers' bills ObserveCraft reads (AWS Cost and Usage Reports,
// Azure cost exports), and the providers that can be connected, plus custom costs files for the others.
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Plus, Trash2 } from 'lucide-react';
import type { BillConfig } from '../../api/cloudCost/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { IntegrationLogo } from '../integrations/IntegrationLogo';
import { useAccess } from '../settings/queries';
import { SetupModal } from '../setup/SetupOptions';
import { billErrors, billSource, billState, monthsRead } from './billSetup';
import { useBillConfigMutations, useBillConfigs } from './billQueries';
import { cloudCostSetup } from './cloudCostSetup';

export const PROVIDERS = {
  aws: { name: 'Amazon Web Services', logo: 'amazonwebservices' },
  azure: { name: 'Microsoft Azure', logo: 'tile:azure' },
  gcp: { name: 'Google Cloud', logo: 'googlecloud' },
} as const;

const AVAILABLE: { id: string; name: string; logo?: string; text: string; to?: string; soon?: boolean }[] = [
  { id: 'aws', ...PROVIDERS.aws, text: 'Read the Cost and Usage Report (CUR 2.0 or classic) your payer account delivers to S3.', to: '/cloud-cost/settings/aws' },
  { id: 'azure', ...PROVIDERS.azure, text: 'Read the actual and amortized cost exports Cost Management writes to a storage account.', to: '/cloud-cost/settings/azure' },
  { id: 'gcp', ...PROVIDERS.gcp, text: 'Reading the Cloud Billing export from BigQuery.', soon: true },
  { id: 'custom', name: 'Custom costs file', text: 'Upload line items from any other provider or SaaS vendor through the Cloud Cost API.' },
];

const STATE_LABEL = { active: 'ACTIVE', disabled: 'DISABLED', error: 'ERROR' } as const;

export function ProvidersPage() {
  const q = useBillConfigs();
  const [setup, setSetup] = useState(false);
  return (
    <div className="ccm-page ccm-providers">
      <h2>Providers</h2>
      {q.isPending ? (
        <Loading label="Loading providers…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <section>
          <h3 className="ccm-providers-head">
            {q.data.length} installed provider{q.data.length === 1 ? '' : 's'}
          </h3>
          {q.data.length === 0 ? (
            <p className="muted-text">No provider bills are read yet. Connect one below; costs appear after the first import.</p>
          ) : (
            <div className="ccm-provider-list">
              {q.data.map((c) => (
                <InstalledProvider key={`${c.provider}:${c.id}`} config={c} />
              ))}
            </div>
          )}
        </section>
      )}
      <section>
        <h3 className="ccm-providers-head">{AVAILABLE.length} available providers</h3>
        <div className="ccm-provider-grid">
          {AVAILABLE.map((p) => (
            <div key={p.id} className={`ccm-provider-card${p.soon ? ' soon' : ''}`}>
              <div className="ccm-provider-card-logo">{p.logo ? <IntegrationLogo logo={p.logo} name={p.name} size={30} /> : <span className="ccm-provider-file">{'{ }'}</span>}</div>
              <strong>{p.name}</strong>
              <p className="muted-text">{p.text}</p>
              {p.soon ? (
                <span className="ccm-badge">Not available yet</span>
              ) : p.to ? (
                <Link className="btn btn-sm" to={p.to}>
                  <Plus size={14} /> Connect
                </Link>
              ) : (
                <button className="btn btn-sm" onClick={() => setSetup(true)}>
                  <Plus size={14} /> Connect
                </button>
              )}
            </div>
          ))}
        </div>
      </section>
      {setup && <SetupModal title="Bring costs into ObserveCraft" options={cloudCostSetup(window.location.origin)} onClose={() => setSetup(false)} />}
    </div>
  );
}

function InstalledProvider({ config: c }: { config: BillConfig }) {
  const { isAdmin } = useAccess();
  const { setEnabled, remove } = useBillConfigMutations();
  const [confirm, setConfirm] = useState(false);
  const p = PROVIDERS[c.provider];
  const state = billState(c);
  const errors = state === 'disabled' ? [] : billErrors(c);
  const source = billSource(c);
  const months = monthsRead(c);
  const failure = (setEnabled.error ?? remove.error) as Error | null;
  return (
    <div className="ccm-provider-row">
      <div className="ccm-provider-main">
        <IntegrationLogo logo={p.logo} name={p.name} size={26} />
        <div className="ccm-provider-what">
          <strong>{p.name}</strong>
          <span className="muted-text" title={`${source.account} · ${source.location}`}>
            <span className="mono">{source.account}</span> · <span className="mono">{source.location}</span>
          </span>
        </div>
        <span className={`ccm-provider-state ${state}`}>{STATE_LABEL[state]}</span>
        <span className="ccm-provider-import muted-text">
          {c.last_import_at ? `Imported ${timeAgo(c.last_import_at)}` : 'Not imported yet'}
          <br />
          {months} month{months === 1 ? '' : 's'} read
        </span>
        <span className="ccm-provider-actions">
          <button className="btn btn-sm" disabled={!isAdmin || setEnabled.isPending} onClick={() => setEnabled.mutate({ provider: c.provider, id: c.id, enabled: state === 'disabled' })}>
            {state === 'disabled' ? 'Enable' : 'Disable'}
          </button>
          <button className="btn btn-sm" disabled={!isAdmin} onClick={() => setConfirm(true)}>
            <Trash2 size={14} /> Remove
          </button>
        </span>
      </div>
      {errors.length > 0 && (
        <ul className="ccm-provider-errors">
          {errors.map((e) => (
            <li key={e}>{e}</li>
          ))}
        </ul>
      )}
      {failure && <p className="form-error">{failure.message}</p>}
      {confirm && (
        <Modal onClose={() => setConfirm(false)} label="Remove provider" width={460} height="auto">
          <div className="small-modal">
            <h3>Stop reading this {p.name} bill?</h3>
            <p className="muted-text">ObserveCraft stops importing {source.location}. The costs already imported stay.</p>
            <div className="small-modal-actions">
              <button className="btn" onClick={() => setConfirm(false)}>
                Cancel
              </button>
              <button className="btn btn-danger" disabled={remove.isPending} onClick={() => remove.mutate({ provider: c.provider, id: c.id }, { onSuccess: () => setConfirm(false) })}>
                {remove.isPending ? 'Removing…' : 'Remove'}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

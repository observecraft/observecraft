import { useState } from 'react';
import { API_MODE } from '../../api/http';
import { Link } from 'react-router-dom';
import { Check, ChevronDown, LayoutDashboard, Plus, Siren, TriangleAlert, X } from 'lucide-react';
import type { IntegrationDetail, IntegrationStatus } from '../../api/integrations/types';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { IntegrationLogo } from './IntegrationLogo';
import { useInstallToggle, useIntegration, useIntegrationDashboards } from './queries';

const TABS = ['Overview', 'Configure', 'Data Collected', 'Hosts', 'Monitoring Resources', 'Support', 'Release Notes'] as const;
type Tab = (typeof TABS)[number];

const STATUS_BADGE: Record<IntegrationStatus, string> = {
  available: 'AVAILABLE',
  detected: 'DETECTED',
  installed: 'INSTALLED',
  warning: 'NEEDS ATTENTION',
};

interface Props {
  id: string;
  onClose: () => void;
  onCategory: (category: string) => void;
}

export function IntegrationModal({ id, onClose, onCategory }: Props) {
  const q = useIntegration(id);
  return (
    <Modal onClose={onClose} label="Integration details">
      {q.isPending ? (
        <Loading label="Loading integration…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <Body detail={q.data} onClose={onClose} onCategory={onCategory} />
      )}
    </Modal>
  );
}

function Body({ detail: d, onClose, onCategory }: { detail: IntegrationDetail; onClose: () => void; onCategory: (c: string) => void }) {
  const [tab, setTab] = useState<Tab>('Overview');
  const toggle = useInstallToggle(d.id);
  const installed = d.status === 'installed' || d.status === 'warning';

  return (
    <>
      <header className="im-header">
        <span className="im-logo">
          <IntegrationLogo logo={d.logo} name={d.name} size={30} />
        </span>
        <div className="im-title">
          <div>
            <h2>{d.name}</h2>
            <span className={`im-badge badge-${d.status}`}>{STATUS_BADGE[d.status]}</span>
          </div>
          <p>
            {d.description} Provided by <a href="#">{d.publisher}</a>.
          </p>
        </div>
        <div className="im-actions">
          {installed ? (
            <div className="btn-group">
              <span className="btn im-installed">
                <Check size={14} /> Installed
              </span>
              <Menu trigger={<ChevronDown size={14} />} items={[{ label: 'Uninstall', onClick: () => toggle.mutate(false) }]} />
            </div>
          ) : (
            <button className="btn btn-primary" disabled={toggle.isPending} onClick={() => setTab('Configure')}>
              <Plus size={14} /> {API_MODE === 'live' ? 'Configure' : 'Install'}
            </button>
          )}
          <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
            <X size={22} />
          </button>
        </div>
      </header>

      {d.status === 'warning' && d.warning && (
        <div className="im-warning">
          <TriangleAlert size={15} /> {d.warning}
        </div>
      )}

      <nav className="im-tabs">
        {TABS.map((t) => (
          <button key={t} className={t === tab ? 'active' : ''} onClick={() => setTab(t)}>
            {t}
          </button>
        ))}
      </nav>

      <div className="im-body">
        {tab === 'Overview' && <Overview d={d} onCategory={onCategory} />}
        {tab === 'Configure' && <Configure d={d} installed={installed} pending={toggle.isPending} onInstall={() => toggle.mutate(true)} />}
        {tab === 'Data Collected' && <DataCollected d={d} />}
        {tab === 'Hosts' && <Hosts d={d} />}
        {tab === 'Monitoring Resources' && <Resources d={d} installed={installed} />}
        {tab === 'Support' && (
          <div className="im-text">
            <p>This integration is built and maintained by {d.publisher}.</p>
            <p>For help, contact your ObserveCraft administrator or open a support ticket with the integration name and the collector version from the Hosts tab.</p>
          </div>
        )}
        {tab === 'Release Notes' && d.releaseNotes.length === 0 && <p className="im-text muted-text">No published release notes for this configuration template.</p>}
        {tab === 'Release Notes' && (
          <div className="im-text">
            {d.releaseNotes.map((r) => (
              <section key={r.version} className="im-release">
                <h4>
                  {r.version} <span className="muted-text">{r.date}</span>
                </h4>
                <ul>
                  {r.notes.map((n) => (
                    <li key={n}>{n}</li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </div>
    </>
  );
}

function Overview({ d, onCategory }: { d: IntegrationDetail; onCategory: (c: string) => void }) {
  return (
    <div className="im-overview">
      <aside className="im-facts">
        <h4>Categories</h4>
        <div className="im-cats">
          {d.categories.map((c) => (
            <button key={c} className="im-cat" onClick={() => onCategory(c)}>
              {c}
            </button>
          ))}
        </div>
        <h4>Offerings</h4>
        {d.offerings.map((o) => (
          <div key={o}>{o}</div>
        ))}
        <h4>Supported OS</h4>
        {d.supportedOs.map((o) => (
          <div key={o}>{o}</div>
        ))}
        <h4>Supported Data Types</h4>
        {d.dataTypes.map((o) => (
          <div key={o}>{o}</div>
        ))}
      </aside>
      <div className="im-text">
        <p>{d.overview}</p>
        <p>
          <strong>Minimum collector version:</strong> {d.minCollectorVersion}
        </p>
      </div>
    </div>
  );
}

function Configure({ d, installed, pending, onInstall }: { d: IntegrationDetail; installed: boolean; pending: boolean; onInstall: () => void }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const missing = d.settings.some((s) => !values[s.key]?.trim());
  return (
    <div className="im-text im-configure">
      <h3>Setup</h3>
      <ol>
        {d.setupSteps.map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ol>

      {API_MODE === 'mock' && d.settings.length > 0 && (
        <div className="im-settings">
          {d.settings.map((s) => (
            <label key={s.key}>
              <span>{s.label}</span>
              <input
                type={s.secret ? 'password' : 'text'}
                placeholder={s.placeholder}
                value={values[s.key] ?? ''}
                onChange={(e) => setValues((v) => ({ ...v, [s.key]: e.target.value }))}
                disabled={installed}
              />
            </label>
          ))}
        </div>
      )}

      {d.configs.map((c) => (
        <CodeBlock
          key={c.filename + c.target}
          code={c.content}
          filename={c.filename}
          badge={<span className={`cfg-target cfg-${c.target}`}>{c.target === 'vector' ? 'Vector' : 'OTel Collector'}</span>}
        />
      ))}
      {d.configs.length > 0 && (
        <p className="muted-text">
          Configuration templates require review for your collector version. Merge the receiver into your collector configuration and add it to the service pipeline before restarting.
        </p>
      )}

      <div className="im-install-row">
        {API_MODE === 'live' ? (
          <p className="muted-text">
            ObserveCraft detects matching metrics received in the last 24 hours. Configure collection on your infrastructure; this page does not deploy or remove collectors.
          </p>
        ) : (
          <>
            {installed ? (
              <span className="im-installed-note">
                <Check size={15} /> Installed. Configuration is rolled out to matching collectors.
              </span>
            ) : (
              <button className="btn btn-primary" disabled={pending || missing} onClick={onInstall} title={missing ? 'Fill in all settings first' : undefined}>
                <Plus size={14} /> {pending ? 'Installing…' : `Install ${d.name}`}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function DataCollected({ d }: { d: IntegrationDetail }) {
  return (
    <div className="im-text">
      <h3>Metrics</h3>
      {d.metrics.length === 0 ? (
        <p className="muted-text">This integration does not collect metrics.</p>
      ) : (
        <table className="im-table">
          <thead>
            <tr>
              <th>METRIC</th>
              <th>TYPE</th>
              <th>UNIT</th>
              <th>DESCRIPTION</th>
            </tr>
          </thead>
          <tbody>
            {d.metrics.map((m) => (
              <tr key={m.name}>
                <td className="mono">{m.name}</td>
                <td>{m.type}</td>
                <td>{m.unit ?? '—'}</td>
                <td>{m.description}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h3>Logs</h3>
      <p className={d.logs ? undefined : 'muted-text'}>{d.logs ?? 'This integration does not collect logs.'}</p>
    </div>
  );
}

function Hosts({ d }: { d: IntegrationDetail }) {
  if (d.hosts.length === 0)
    return (
      <p className="im-text muted-text">
        {API_MODE === 'live'
          ? 'No verified host inventory is available for this integration.'
          : d.status === 'available'
            ? 'No hosts are running this integration.'
            : 'This integration is not host-based (it runs through the provider API).'}
      </p>
    );
  return (
    <div className="im-text">
      <table className="im-table">
        <thead>
          <tr>
            <th>HOST</th>
            <th>STATUS</th>
            <th>COLLECTOR</th>
            <th>LAST REPORTED</th>
          </tr>
        </thead>
        <tbody>
          {d.hosts.map((h) => (
            <tr key={h.host}>
              <td className="mono">{h.host}</td>
              <td>
                <span className={`host-status hs-${h.status}`}>{h.status.toUpperCase()}</span>
                {h.message && <div className="muted-text host-msg">{h.message}</div>}
              </td>
              <td className="mono">{h.collectorVersion}</td>
              <td>{timeAgo(h.lastReported)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Resources({ d, installed }: { d: IntegrationDetail; installed: boolean }) {
  const dashboards = useIntegrationDashboards(d.id);
  if (d.resources.length === 0 && !dashboards.isFetching && !dashboards.isError && !dashboards.data?.length)
    return <p className="im-text muted-text">No dashboards or monitors ship with this integration.</p>;
  return (
    <div className="im-text">
      <ul className="im-resources">
        {dashboards.isFetching && <li>Loading dashboards…</li>}
        {dashboards.isError && (
          <li>
            <ErrorState error={dashboards.error} onRetry={() => dashboards.refetch()} />
          </li>
        )}
        {dashboards.data?.map((dashboard) => (
          <li key={dashboard.id}>
            <LayoutDashboard size={16} />
            <div>
              <strong>{dashboard.title}</strong>
              <div className="muted-text">Bundled integration dashboard. Collect metrics to populate its charts.</div>
            </div>
            <Link className="btn" to={`/dashboards/${dashboard.id}`}>
              Open dashboard
            </Link>
          </li>
        ))}
        {d.resources
          .filter((r) => r.kind !== 'dashboard')
          .map((r) => (
            <li key={r.name}>
              {r.kind === 'dashboard' ? <LayoutDashboard size={16} /> : <Siren size={16} />}
              <div>
                <strong>{r.name}</strong>
                <div className="muted-text">{r.description}</div>
              </div>
              <button className="btn" disabled={!installed} title={installed ? undefined : 'Install the integration first'}>
                {r.kind === 'dashboard' ? 'Open dashboard' : 'Create monitor'}
              </button>
            </li>
          ))}
      </ul>
    </div>
  );
}

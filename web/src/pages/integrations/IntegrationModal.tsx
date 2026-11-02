import { useMemo, useState } from 'react';
import { API_MODE } from '../../api/http';
import { Link } from 'react-router-dom';
import { BookOpen, Check, ChevronDown, ExternalLink, LayoutDashboard, Newspaper, Plus, Search, Siren, TriangleAlert, X } from 'lucide-react';
import type { IntegrationDocs } from '../../api/integrations/catalog';
import type { IntegrationDetail, IntegrationStatus } from '../../api/integrations/types';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { AgentConfigForm } from './AgentConfigForm';
import { IntegrationLogo } from './IntegrationLogo';
import { useInstallToggle, useIntegration, useIntegrationDashboards, useIntegrationDocs } from './queries';
import { ReadmeMarkdown } from './ReadmeMarkdown';

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
  const docs = useIntegrationDocs(d.coreId).data ?? null;
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
            {/[.!?]$/.test(d.description) ? d.description : `${d.description}.`} Provided by <a href="#">{d.publisher}</a>.
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
        {tab === 'Overview' && <Overview d={d} docs={docs} onCategory={onCategory} />}
        {tab === 'Configure' && <Configure d={d} docs={docs} installed={installed} pending={toggle.isPending} onInstall={() => toggle.mutate(true)} />}
        {tab === 'Data Collected' && <DataCollected d={d} docs={docs} />}
        {tab === 'Hosts' && <Hosts d={d} />}
        {tab === 'Monitoring Resources' && <Resources d={d} docs={docs} installed={installed} />}
        {tab === 'Support' && (
          <div className="im-text">
            {docs?.troubleshooting && <ReadmeMarkdown source={docs.troubleshooting} />}
            {docs?.furtherReading && (
              <>
                <h3>Further Reading</h3>
                <ReadmeMarkdown source={docs.furtherReading} />
              </>
            )}
            {!docs?.troubleshooting && (
              <>
                <p>This integration is built and maintained by {d.publisher}.</p>
                <p>For help, contact your ObserveCraft administrator or open a support ticket with the integration name and the collector version from the Hosts tab.</p>
              </>
            )}
          </div>
        )}
        {tab === 'Release Notes' && docs?.releaseNotes.length ? (
          <div className="im-text">
            {docs.releaseNotes.map((r) => (
              <section key={r.version} className="im-release">
                <h3>
                  {r.version}
                  {r.date && ` / ${r.date}`}
                </h3>
                <ReadmeMarkdown source={r.notes} />
              </section>
            ))}
          </div>
        ) : null}
        {tab === 'Release Notes' && !docs?.releaseNotes.length && d.releaseNotes.length === 0 && <p className="im-text muted-text">No published release notes for this configuration template.</p>}
        {tab === 'Release Notes' && !docs?.releaseNotes.length && (
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

const RESOURCE_LABEL: Record<string, string> = { documentation: 'Documentation', blog: 'Blog post', other: 'Resource' };

function Overview({ d, docs, onCategory }: { d: IntegrationDetail; docs: IntegrationDocs | null; onCategory: (c: string) => void }) {
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
        {docs?.overview ? <ReadmeMarkdown source={docs.overview} /> : <p>{d.overview}</p>}
        {docs && docs.resources.length > 0 && (
          <>
            <h3>Dive Deeper</h3>
            <p className="muted-text">Resources and guides to help you get the most out of {d.name}.</p>
            <div className="im-dive">
              {docs.resources.map((r) => (
                <a key={r.url} className="im-dive-card" href={r.url} target="_blank" rel="noreferrer noopener">
                  {r.type === 'blog' ? <Newspaper size={16} /> : <BookOpen size={16} />}
                  <span className="im-dive-kind">{(RESOURCE_LABEL[r.type] ?? r.type).toUpperCase()}</span>
                  <span className="im-dive-host">{new URL(r.url).hostname.replace(/^www\./, '')}</span>
                  <span className="im-dive-path">{decodeURIComponent(new URL(r.url).pathname.split('/').filter(Boolean).pop() ?? '').replace(/[-_]/g, ' ')}</span>
                </a>
              ))}
            </div>
          </>
        )}
        {d.configs.length > 0 && (
          <p>
            <strong>Minimum collector version:</strong> {d.minCollectorVersion}
          </p>
        )}
      </div>
    </div>
  );
}

function Configure({ d, docs, installed, pending, onInstall }: { d: IntegrationDetail; docs: IntegrationDocs | null; installed: boolean; pending: boolean; onInstall: () => void }) {
  const [values, setValues] = useState<Record<string, string>>({});
  const missing = d.settings.some((s) => !values[s.key]?.trim());
  return (
    <div className="im-text im-configure">
      {docs?.setup ? (
        <>
          <div className="im-observecraft">
            <strong>Sending to ObserveCraft</strong>
            <ol>
              {d.setupSteps.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ol>
          </div>
          <h3>Configuration</h3>
          <ReadmeMarkdown source={docs.setup} />
        </>
      ) : (
        <>
          <h3>Setup</h3>
          <ol>
            {d.setupSteps.map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ol>
        </>
      )}

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

      {d.configureUrl && (
        <div>
          <Link className="btn btn-primary" to={d.configureUrl}>
            Configure {d.categories.includes('AWS') ? 'the AWS integration' : 'the integration'}
          </Link>
        </div>
      )}
      {d.agent?.configurable && <AgentConfigForm check={d.agent.check} />}

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

const PAGE = 10;

/** Metrics of the integration: searchable, ten per page, as Datadog's Data Collected tab. */
function MetricsTable({ metrics }: { metrics: { name: string; unit?: string; perUnit?: string; description: string }[] }) {
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? metrics.filter((m) => m.name.toLowerCase().includes(q) || m.description.toLowerCase().includes(q)) : metrics;
  }, [metrics, query]);
  const pages = Math.max(1, Math.ceil(shown.length / PAGE));
  const current = Math.min(page, pages - 1);
  return (
    <>
      <div className="im-table-bar">
        <span>
          Showing{' '}
          <strong>
            {shown.length ? current * PAGE + 1 : 0}–{Math.min(shown.length, (current + 1) * PAGE)}
          </strong>{' '}
          of <strong>{shown.length}</strong> metrics
        </span>
        <label className="im-search">
          <Search size={14} />
          <input
            placeholder="Search"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setPage(0);
            }}
          />
        </label>
      </div>
      <table className="im-table">
        <thead>
          <tr>
            <th>NAME</th>
            <th>UNITS</th>
            <th>DESCRIPTION</th>
          </tr>
        </thead>
        <tbody>
          {shown.slice(current * PAGE, (current + 1) * PAGE).map((m) => (
            <tr key={m.name}>
              <td className="mono">{m.name}</td>
              <td>{m.unit ? `${m.unit}${m.perUnit ? `/${m.perUnit}` : ''}` : ''}</td>
              <td className="im-desc" title={m.description}>
                {m.description}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {pages > 1 && (
        <div className="im-pager">
          <button className="btn" disabled={current === 0} onClick={() => setPage(current - 1)}>
            Previous
          </button>
          <span className="muted-text">
            Page {current + 1} of {pages}
          </span>
          <button className="btn" disabled={current >= pages - 1} onClick={() => setPage(current + 1)}>
            Next
          </button>
        </div>
      )}
    </>
  );
}

function DataCollected({ d, docs }: { d: IntegrationDetail; docs: IntegrationDocs | null }) {
  const metrics = docs?.metrics.length ? docs.metrics : d.metrics;
  const source = d.logs?.match(/source:(\S+?)\.?$/)?.[1];
  return (
    <div className="im-text">
      <h3>Logs</h3>
      {docs?.logFacets.length ? (
        <>
          <div className="im-table-bar">
            <span>The following log attributes are tracked and parsed into searchable facets:</span>
            <Link className="btn" to={`/logs?query=${encodeURIComponent(`source:${source ?? d.coreId ?? d.id}`)}`}>
              View Logs in Log Explorer <ExternalLink size={13} />
            </Link>
          </div>
          <table className="im-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>PATH</th>
                <th>FACET TYPE</th>
                <th>GROUP</th>
              </tr>
            </thead>
            <tbody>
              {docs.logFacets.map((f) => (
                <tr key={f.path}>
                  <td>{f.name}</td>
                  <td className="mono">{f.path}</td>
                  <td>{f.facetType}</td>
                  <td>{f.group}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : (
        <p className={d.logs ? undefined : 'muted-text'}>{d.logs ?? 'This integration does not collect logs.'}</p>
      )}
      <h3>Metrics</h3>
      {metrics.length === 0 ? <p className="muted-text">This integration does not collect metrics.</p> : <MetricsTable metrics={metrics} />}
      {docs && docs.serviceChecks.length > 0 && (
        <>
          <h3>Service Checks</h3>
          <table className="im-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>STATUSES</th>
                <th>DESCRIPTION</th>
              </tr>
            </thead>
            <tbody>
              {docs.serviceChecks.map((c) => (
                <tr key={c.check}>
                  <td className="mono">{c.check}</td>
                  <td>{c.statuses.join(', ')}</td>
                  <td>{c.description}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
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

function Resources({ d, docs, installed }: { d: IntegrationDetail; docs: IntegrationDocs | null; installed: boolean }) {
  const dashboards = useIntegrationDashboards(d.coreId ? [d.id, d.coreId] : [d.id]);
  const opened = (title: string) => dashboards.data?.find((x) => x.title === title);
  const monitors = d.resources.filter((r) => r.kind !== 'dashboard');
  if (!docs?.dashboards.length && !docs?.monitors.length && !monitors.length && !dashboards.data?.length && !dashboards.isFetching)
    return <p className="im-text muted-text">No dashboards or monitors ship with this integration.</p>;
  return (
    <div className="im-text">
      <h3>Dashboards</h3>
      {dashboards.isError && <ErrorState error={dashboards.error} onRetry={() => dashboards.refetch()} />}
      <table className="im-table">
        <thead>
          <tr>
            <th>NAME</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(docs?.dashboards.map((x) => x.title) ?? dashboards.data?.map((x) => x.title) ?? []).map((title) => {
            const dashboard = opened(title);
            return (
              <tr key={title}>
                <td>
                  <LayoutDashboard size={14} /> {title}
                </td>
                <td className="im-row-action">
                  {dashboard ? (
                    <Link to={`/dashboards/${dashboard.id}`}>Open dashboard</Link>
                  ) : (
                    <span className="muted-text">{dashboards.isFetching ? 'Loading…' : 'Available once the integration sends data'}</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {(docs?.monitors.length || monitors.length) > 0 && (
        <>
          <h3>Monitor Templates</h3>
          <table className="im-table">
            <thead>
              <tr>
                <th>NAME</th>
                <th>TYPE</th>
              </tr>
            </thead>
            <tbody>
              {docs?.monitors.map((m) => (
                <tr key={m.title} title={m.description}>
                  <td>{m.title}</td>
                  <td>
                    <Siren size={14} /> {m.type}
                  </td>
                </tr>
              ))}
              {monitors.map((r) => (
                <tr key={r.name} title={r.description}>
                  <td>{r.name}</td>
                  <td>
                    <Siren size={14} /> {installed ? 'Monitor' : 'Monitor (install first)'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}

import { useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { Bug, Check, CheckCircle2, ChevronDown, ChevronRight, Code2, Copy, Gauge, Monitor, Settings, Sparkles, Trash2, Users, X } from 'lucide-react';
import type { RumApplication } from '../../api/rum/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { AppTypeIcon, TYPE_LABEL, isMobile } from './common';
import { useDeleteRumApplication, useRumApplication, useRumApplications, useRumEvents, useSaveRumApplication } from './queries';
import { RumCrumbs } from './RumApplicationsPage';
import '../monitors/monitors.css';
import './rum.css';

type Tab = 'sdk' | 'verification' | 'ownership' | 'rum' | 'errors' | 'analytics';
const CONFIG: [Tab, string, typeof Code2][] = [
  ['sdk', 'SDK Installation', Code2],
  ['verification', 'Verification', Check],
  ['ownership', 'Ownership', Users],
];
const PRODUCTS: [Tab, string, typeof Code2][] = [
  ['rum', 'Real User Monitoring', Monitor],
  ['errors', 'Error Tracking', Bug],
  ['analytics', 'Product Analytics', Sparkles],
];

const INTAKE = `${window.location.protocol}//${window.location.hostname}:4318`;
const HOUR = 3_600_000;

/** A RUM application: its SDK setup, verification, ownership and product settings. */
export function RumApplicationPage() {
  const id = useParams().id!;
  const q = useRumApplication(id);
  if (q.isPending) return <Loading label="Loading the application…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <ApplicationView app={q.data} />;
}

function ApplicationView({ app }: { app: RumApplication }) {
  const navigate = useNavigate();
  const apps = useRumApplications();
  const [params, setParams] = useSearchParams();
  const tab = ([...CONFIG, ...PRODUCTS].find(([t]) => t === params.get('tab'))?.[0] ?? 'rum') as Tab;
  const setTab = (t: Tab) => setParams({ tab: t }, { replace: true });
  const [editing, setEditing] = useState(false);
  const [copied, setCopied] = useState(false);
  const [now] = useState(() => Date.now());
  const recent = useRumEvents({ range: { from: now - 24 * HOUR, to: now }, query: '', eventType: 'all', applicationId: app.id, limit: 1 });
  const receiving = (recent.data?.total ?? 0) > 0;
  const copy = () => navigator.clipboard?.writeText(app.id).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1200)));
  const rumOn = app.rumEventProcessingState !== 'NONE';
  const analyticsOn = app.productAnalyticsRetentionState === 'MAX';

  return (
    <div className="mon-status-page">
      <RumCrumbs>
        <ChevronRight size={18} className="muted" />
        <Menu
          align="left"
          triggerClassName="rum-crumb-app"
          trigger={
            <>
              <AppTypeIcon type={app.type} size={18} /> {app.name} <ChevronDown size={16} />
            </>
          }
          items={(apps.data ?? []).map((a) => ({ label: a.name, onClick: () => navigate(`/rum/applications/${a.id}?tab=${tab}`) }))}
        />
      </RumCrumbs>
      <div className="mon-status-scroll">
        <section className="rum-app-head">
          <div>
            <h1>
              <AppTypeIcon type={app.type} size={34} /> {app.name}
              <button className="icon-plain" title={copied ? 'Copied' : `Copy the application ID (${app.id})`} onClick={copy}>
                <Copy size={16} />
              </button>
            </h1>
            <p className="muted">
              Created by <b>{app.createdBy ?? 'someone'}</b> on {new Date(app.createdAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}
              <span className="syn-meta-sep" />
              <button className="link-btn" onClick={() => setEditing(true)}>
                <Settings size={14} /> Edit Application
              </button>
            </p>
          </div>
          <div className="rum-app-products">
            <button className={`rum-product-card${rumOn ? ' on' : ''}`} onClick={() => setTab('rum')}>
              <Gauge size={30} />
              <span>
                <b>Real User Monitoring</b>
                <span className="muted">Optimize app performance</span>
              </span>
            </button>
            <button className={`rum-product-card${analyticsOn ? ' on' : ''}`} onClick={() => setTab('analytics')}>
              <Sparkles size={30} />
              <span>
                <b>Product Analytics</b>
                <span className="muted">Product trends &amp; user behavior</span>
              </span>
            </button>
          </div>
        </section>
        <div className="rum-app-body">
          <nav className="rum-app-nav">
            <h5>CONFIGURATION</h5>
            {CONFIG.map(([t, label, Icon]) => (
              <button key={t} className={tab === t ? 'active' : ''} onClick={() => setTab(t)}>
                <Icon size={16} /> {label}
                {t === 'verification' && receiving && <CheckCircle2 size={15} className="rum-ok" />}
                {t === 'sdk' && receiving && <CheckCircle2 size={15} className="rum-ok" />}
              </button>
            ))}
            <h5>PRODUCT SETTINGS</h5>
            {PRODUCTS.map(([t, label, Icon]) => (
              <button key={t} className={tab === t ? 'active' : ''} onClick={() => setTab(t)}>
                <Icon size={16} /> {label}
                {((t === 'rum' && rumOn) || (t === 'analytics' && analyticsOn) || (t === 'errors' && rumOn)) && <CheckCircle2 size={15} className="rum-ok" />}
              </button>
            ))}
          </nav>
          <div className="rum-app-panel">
            {tab === 'sdk' && <SdkInstallation app={app} />}
            {tab === 'verification' && <Verification app={app} receiving={receiving} lastAt={recent.data?.rows[0]?.timestamp} loading={recent.isPending} />}
            {tab === 'ownership' && <Ownership app={app} />}
            {tab === 'rum' && <RumSettings app={app} />}
            {tab === 'errors' && <ErrorTrackingSettings app={app} />}
            {tab === 'analytics' && <AnalyticsSettings app={app} />}
          </div>
        </div>
      </div>
      {editing && <EditApplication app={app} onClose={() => setEditing(false)} />}
    </div>
  );
}

function Snippet({ title, code }: { title: string; code: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="rum-snippet">
      <div className="rum-snippet-head">
        <span>{title}</span>
        <button className="link-btn" onClick={() => navigator.clipboard?.writeText(code).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1200)))}>
          <Copy size={13} /> {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre>{code}</pre>
    </div>
  );
}

function SdkInstallation({ app }: { app: RumApplication }) {
  const [method, setMethod] = useState<'npm' | 'cdn'>('npm');
  const init = `datadogRum.init({
  applicationId: '${app.id}',
  clientToken: '${app.clientToken}',
  // Send to ObserveCraft's intake instead of Datadog.
  proxy: '${INTAKE}/api/v2/rum',
  service: '${app.name}',
  env: 'prod',
  version: '1.0.0',
  sessionSampleRate: 100,
  trackUserInteractions: true,
  trackResources: true,
  trackLongTasks: true,
  defaultPrivacyLevel: 'mask-user-input',
});`;
  if (isMobile(app.type)) {
    return (
      <section>
        <h2 className="rum-panel-title">SDK Installation</h2>
        <p className="muted">
          Add Datadog&rsquo;s {TYPE_LABEL[app.type]} RUM SDK to the app and initialize it with the application ID and client token below. Point its custom endpoint (the SDK&rsquo;s{' '}
          <code>useCustomEndpoints</code> / <code>rumEndpointUrl</code> option) at <code>{INTAKE}/api/v2/rum</code>.
        </p>
        <Credentials app={app} />
      </section>
    );
  }
  return (
    <section>
      <h2 className="rum-panel-title">SDK Installation</h2>
      <p className="muted">Datadog&rsquo;s browser SDK (Apache-2.0) collects sessions, views, actions, resources, errors and long tasks. Its proxy option sends them to ObserveCraft.</p>
      <Credentials app={app} />
      <div className="syn-seg rum-method">
        <button className={method === 'npm' ? 'active' : ''} onClick={() => setMethod('npm')}>
          npm
        </button>
        <button className={method === 'cdn' ? 'active' : ''} onClick={() => setMethod('cdn')}>
          CDN async
        </button>
      </div>
      {method === 'npm' ? (
        <>
          <Snippet title="1. Install the SDK" code="npm install --save @datadog/browser-rum" />
          <Snippet title="2. Initialize it as early as possible in your application" code={`import { datadogRum } from '@datadog/browser-rum';\n\n${init}`} />
        </>
      ) : (
        <Snippet
          title="Add this to the <head> of every page"
          code={`<script>
  (function(h,o,u,n,d) {
    h=h[d]=h[d]||{q:[],onReady:function(c){h.q.push(c)}}
    d=o.createElement(u);d.async=1;d.src=n
    n=o.getElementsByTagName(u)[0];n.parentNode.insertBefore(d,n)
  })(window,document,'script','https://www.datadoghq-browser-agent.com/us1/v6/datadog-rum.js','DD_RUM')
  window.DD_RUM.onReady(function() {
    window.DD_RUM.${init.replace(/\n/g, '\n    ')}
  })
</script>`}
        />
      )}
    </section>
  );
}

function Credentials({ app }: { app: RumApplication }) {
  return (
    <div className="mon-details syn-details rum-creds">
      <div>
        <span className="muted">Application ID</span>
        <span className="mono">{app.id}</span>
      </div>
      <div>
        <span className="muted">Client token</span>
        <span className="mono">{app.clientToken}</span>
      </div>
      <div>
        <span className="muted">Intake</span>
        <span className="mono">{INTAKE}/api/v2/rum</span>
      </div>
    </div>
  );
}

function Verification({ app, receiving, lastAt, loading }: { app: RumApplication; receiving: boolean; lastAt?: number; loading: boolean }) {
  return (
    <section>
      <h2 className="rum-panel-title">Verification</h2>
      <div className={`rum-status-card${receiving ? ' ok' : ''}`}>
        {loading ? (
          <span className="muted">Checking for events…</span>
        ) : receiving ? (
          <>
            <CheckCircle2 size={18} /> {app.name} is sending events: the latest arrived {lastAt ? timeAgo(lastAt) : 'recently'}.
          </>
        ) : (
          <>No events in the last 24 hours. Once the SDK is installed and the application is used, its sessions appear here within a minute.</>
        )}
      </div>
      {receiving && (
        <p>
          <Link to={`/rum/explorer?application=${app.id}`}>See the sessions in the RUM Explorer</Link>
        </p>
      )}
    </section>
  );
}

function Ownership({ app }: { app: RumApplication }) {
  const save = useSaveRumApplication(app.id);
  const [tags, setTags] = useState(app.tags.join(', '));
  const list = tags
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);
  return (
    <section>
      <h2 className="rum-panel-title">Ownership</h2>
      <p className="muted">Tag the application with the team that owns it (team:storefront), and any other tags to find it by.</p>
      <label className="rum-field">
        <span>Tags</span>
        <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="team:storefront, tier:critical" />
      </label>
      <button className="btn btn-primary" disabled={save.isPending || list.join(',') === app.tags.join(',')} onClick={() => save.mutate({ tags: list })}>
        {save.isPending ? 'Saving…' : 'Save'}
      </button>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
    </section>
  );
}

const PROCESSING: [string, string, string][] = [
  ['ALL', 'All sessions', 'Every session the SDKs send is kept.'],
  ['ERROR_FOCUSED_MODE', 'Sessions with errors', 'Meant to keep only sessions that had an error; ObserveCraft keeps every session in this mode for now.'],
  ['NONE', 'No sessions', 'Events are dropped: Real User Monitoring is disabled.'],
];

function RumSettings({ app }: { app: RumApplication }) {
  const save = useSaveRumApplication(app.id);
  const on = app.rumEventProcessingState !== 'NONE';
  return (
    <section>
      <h2 className="rum-panel-title">Real User Monitoring</h2>
      <div className={`rum-status-card${on ? ' ok' : ''} rum-status-bar`}>
        <span>
          {on ? <CheckCircle2 size={18} /> : <X size={18} />} Real User Monitoring is {on ? 'enabled' : 'disabled'}
        </span>
        <button className="btn" disabled={save.isPending} onClick={() => save.mutate({ rumEventProcessingState: on ? 'NONE' : 'ALL' })}>
          {on ? 'Disable' : 'Enable'}
        </button>
        <small className="muted">
          {on ? 'Enabled' : 'Disabled'} on {new Date(on ? app.createdAt : app.rumEventProcessingModifiedAt).toLocaleString()}
        </small>
      </div>
      <h5 className="rum-settings-h">PRODUCT SETTINGS</h5>
      <div className="rum-setting">
        <Users size={18} />
        <div>
          <b>User Sessions Collected by SDKs</b>
          <p className="muted">The SDKs collect sessions with their views, actions, resources, errors and long tasks, as real users experience the application.</p>
          <div className="rum-choices">
            {PROCESSING.map(([state, label, help]) => (
              <label key={state} className={`rum-choice${app.rumEventProcessingState === state ? ' active' : ''}`}>
                <input type="radio" name="processing" checked={app.rumEventProcessingState === state} disabled={save.isPending} onChange={() => save.mutate({ rumEventProcessingState: state })} />
                <span>
                  <b>{label}</b>
                  <span className="muted">{help}</span>
                </span>
              </label>
            ))}
          </div>
        </div>
      </div>
      <div className="rum-setting">
        <Gauge size={18} />
        <div>
          <b>Performance Metrics</b>
          <p className="muted">Loading time and Core Web Vitals (LCP, INP, CLS) of every view, at the 75th percentile on the Summary page.</p>
        </div>
      </div>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
    </section>
  );
}

function ErrorTrackingSettings({ app }: { app: RumApplication }) {
  return (
    <section>
      <h2 className="rum-panel-title">Error Tracking</h2>
      <p className="muted">The application&rsquo;s errors are in the RUM Explorer; JavaScript source maps and mobile debug symbols can&rsquo;t be uploaded yet, so stack traces stay minified.</p>
      <Link className="btn" to={`/rum/explorer?application=${app.id}&type=error`}>
        See the errors
      </Link>
    </section>
  );
}

function AnalyticsSettings({ app }: { app: RumApplication }) {
  const save = useSaveRumApplication(app.id);
  const on = app.productAnalyticsRetentionState === 'MAX';
  return (
    <section>
      <h2 className="rum-panel-title">Product Analytics</h2>
      <div className={`rum-status-card${on ? ' ok' : ''} rum-status-bar`}>
        <span>
          {on ? <CheckCircle2 size={18} /> : <X size={18} />} Product Analytics retention is {on ? 'enabled' : 'disabled'}
        </span>
        <button className="btn" disabled={save.isPending} onClick={() => save.mutate({ productAnalyticsRetentionState: on ? 'NONE' : 'MAX' })}>
          {on ? 'Disable' : 'Enable'}
        </button>
      </div>
      <p className="muted">Keeps the application&rsquo;s sessions for product trends and user behavior analysis.</p>
    </section>
  );
}

function EditApplication({ app, onClose }: { app: RumApplication; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSaveRumApplication(app.id);
  const remove = useDeleteRumApplication();
  const [name, setName] = useState(app.name);
  return (
    <Modal onClose={onClose} label="Edit Application" width={520} height="auto">
      <div className="syn-dialog">
        <h3>Edit Application</h3>
        <label className="form-label">
          Name
          <input className="syn-input" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        <p className="muted">
          Type: {TYPE_LABEL[app.type] ?? app.type} · Client token <span className="mono">{app.clientToken}</span>
        </p>
        {(save.error || remove.error) && <p className="form-error">{((save.error ?? remove.error) as Error).message}</p>}
        <div className="syn-dialog-actions">
          <button
            className="btn syn-danger rum-delete"
            disabled={remove.isPending}
            onClick={() =>
              window.confirm(`Delete ${app.name}? Its client token stops working; the events already sent stay until they expire.`) &&
              remove.mutate(app.id, { onSuccess: () => navigate('/rum/applications') })
            }
          >
            <Trash2 size={14} /> Delete
          </button>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending} onClick={() => save.mutate({ name: name.trim() }, { onSuccess: onClose })}>
            {save.isPending ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

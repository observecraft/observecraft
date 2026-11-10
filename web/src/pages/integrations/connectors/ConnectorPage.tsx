// Integrations → a connector (docs/connectors.md): one page for every connector package, drawn from its manifest. The
// setup form comes from its config fields (secrets masked, kept when left empty), the streams can be turned on and
// off, and each connection shows its streams' last runs. Push-only packages show where to point the vendor instead.
// Native connectors have their own pages: this one sends there.
import { useMemo, useState } from 'react';
import { Link, Navigate, useParams, useSearchParams } from 'react-router-dom';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { AlertTriangle, Check, ChevronRight, CircleCheck, Plus, RefreshCw } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  connectorPage,
  createConnection,
  createPush,
  deleteConnection,
  getConnector,
  syncConnection,
  testConnection,
  updateConnection,
  type ConnectionInput,
  type ConnectionView,
  type ConnectorEntry,
  type ConnectorField,
} from '../../../api/connectors/client';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { timeAgo } from '../../../lib/format';
import { useAccess } from '../../settings/queries';
import { IntegrationLogo } from '../IntegrationLogo';
import { useIntegrationDashboards } from '../queries';
import { Step } from '../Step';
import '../../settings/settings.css';
import '../integrations.css';
import '../aws/aws.css';
import '../../delivery/delivery.css';

const TABS = [
  { id: 'configure', label: 'Configuration' },
  { id: 'data', label: 'Data Collected' },
] as const;
type Tab = (typeof TABS)[number]['id'];

/** The intake's address as the browser reaches this install (port 4318), as the setup pages show it. */
const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

/** `{{ config.x }}`, `{{ intake }}`, `{{ source }}` parts of a link or an instruction. */
function fill(text: string, vars: Record<string, unknown>): string {
  return text.replace(/\{\{\s*([a-z_.]+)\s*\}\}/g, (_, path: string) => {
    const v = path.split('.').reduce<unknown>((o, k) => (o && typeof o === 'object' ? (o as Record<string, unknown>)[k] : undefined), vars);
    return v === undefined || v === null ? '' : String(v);
  });
}

export function ConnectorPage() {
  const { id = '' } = useParams();
  const [params, setParams] = useSearchParams();
  const tab: Tab = TABS.find((t) => t.id === params.get('tab'))?.id ?? 'configure';
  const detail = useQuery({ queryKey: ['connectors', id], queryFn: () => getConnector(id), refetchInterval: (q) => (q.state.data?.connections.length ? 15_000 : false) });
  if (detail.isPending) return <Loading />;
  if (detail.isError) return <ErrorState error={detail.error} onRetry={() => detail.refetch()} />;
  const c = detail.data.connector;
  if (c.kind === 'native' && c.page) return <Navigate to={c.page} replace />;
  const installed = detail.data.connections.length > 0;
  return (
    <div className="aws-page">
      <div className="product-title rt-crumb aws-crumbs">
        <Link to="/integrations">Integrations</Link>
        <ChevronRight size={18} className="rt-crumb-sep" />
        <span>{c.name}</span>
      </div>
      <header className="aws-hero">
        <IntegrationLogo logo={c.logo} name={c.name} size={72} />
        <div>
          <h1>{c.name}</h1>
          <p>{c.description}</p>
          <p className="muted">
            {c.thirdParty ? (
              <>
                <span className="badge">Third-party</span> Published by {c.publisher}, version {c.version}.{' '}
              </>
            ) : (
              <>By {c.publisher}. </>
            )}
            {c.docsUrl && (
              <a href={c.docsUrl} target="_blank" rel="noreferrer">
                {c.vendor} API documentation
              </a>
            )}
          </p>
        </div>
        {installed && (
          <span className="btn aws-installed">
            <Check size={15} /> Installed
          </span>
        )}
      </header>
      <nav className="im-tabs aws-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={tab === t.id ? 'active' : ''} onClick={() => setParams(t.id === 'configure' ? {} : { tab: t.id })}>
            {t.label}
          </button>
        ))}
      </nav>
      {tab === 'data' ? <DataCollected c={c} /> : c.kind === 'push' ? <Push c={c} connections={detail.data.connections} /> : <Configure c={c} connections={detail.data.connections} />}
    </div>
  );
}

/** The org's dashboards of the connector (their ids carry the org), once it is connected. */
function useDashboardLinks(c: ConnectorEntry) {
  const dashboards = useIntegrationDashboards([c.logo, ...c.tiles]);
  return dashboards.data ?? [];
}

type Form = { name: string; config: Record<string, string | boolean>; secrets: Record<string, string>; streams: string[] };

function emptyForm(c: ConnectorEntry): Form {
  const config: Record<string, string | boolean> = {};
  for (const f of c.config.filter((f) => !f.secret)) {
    const d: unknown = f.default;
    config[f.key] = f.type === 'boolean' ? d === true : Array.isArray(d) ? (d as unknown[]).join('\n') : d === null || d === undefined ? '' : String(d);
  }
  return { name: '', config, secrets: {}, streams: c.streams.filter((s) => s.defaultEnabled).map((s) => s.id) };
}

function formOf(c: ConnectorEntry, v: ConnectionView): Form {
  const f = emptyForm(c);
  for (const [k, val] of Object.entries(v.config ?? {})) f.config[k] = typeof val === 'boolean' ? val : Array.isArray(val) ? val.join('\n') : String(val);
  return { ...f, name: v.name, streams: v.streams.filter((s) => s.enabled).map((s) => s.id) };
}

function inputOf(c: ConnectorEntry, form: Form, connectionId?: string): ConnectionInput {
  const config: Record<string, unknown> = {};
  for (const f of c.config.filter((f) => !f.secret)) {
    const v = form.config[f.key];
    if (f.type === 'list' || f.type === 'multiselect')
      config[f.key] = String(v ?? '')
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean);
    else if (f.type === 'boolean') config[f.key] = !!v;
    else if (f.type === 'number') config[f.key] = v === '' ? null : Number(v);
    else config[f.key] = v === '' ? null : v;
  }
  const secrets = Object.fromEntries(Object.entries(form.secrets).filter(([, v]) => v.trim()));
  return { name: form.name, config, secrets, streams: form.streams, connectionId: connectionId ?? null } as ConnectionInput;
}

function Field({ f, value, saved, disabled, onChange }: { f: ConnectorField; value: string | boolean; saved: boolean; disabled: boolean; onChange: (v: string | boolean) => void }) {
  const text = typeof value === 'string' ? value : '';
  return (
    <label className="rt-field">
      <span>
        {f.label}
        {f.required && !saved ? ' *' : ''}
      </span>
      {f.type === 'boolean' ? (
        <input type="checkbox" checked={!!value} onChange={(e) => onChange(e.target.checked)} disabled={disabled} />
      ) : f.type === 'select' ? (
        <select value={text} onChange={(e) => onChange(e.target.value)} disabled={disabled}>
          <option value="">—</option>
          {f.options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      ) : f.type === 'text' || f.type === 'list' || f.type === 'multiselect' ? (
        <textarea rows={f.secret ? 6 : 3} value={text} onChange={(e) => onChange(e.target.value)} placeholder={saved ? 'Saved (leave empty to keep it)' : (f.placeholder ?? '')} disabled={disabled} />
      ) : (
        <input
          type={f.secret ? 'password' : f.type === 'number' ? 'number' : 'text'}
          value={text}
          onChange={(e) => onChange(e.target.value)}
          autoComplete={f.secret ? 'new-password' : 'off'}
          placeholder={saved ? 'Saved (leave empty to keep it)' : (f.placeholder ?? '')}
          disabled={disabled}
        />
      )}
      {(f.help || f.secret) && <span className="muted">{[f.help, f.secret ? 'Stored encrypted and never shown again.' : ''].filter(Boolean).join(' ')}</span>}
    </label>
  );
}

function Configure({ c, connections }: { c: ConnectorEntry; connections: ConnectionView[] }) {
  const qc = useQueryClient();
  const { isAdmin } = useAccess();
  // The catalog's tile is installed while a connection is saved.
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ['connectors', c.id] }), qc.invalidateQueries({ queryKey: ['integrations'] })]);
  const [editing, setEditing] = useState<string | null>(connections[0]?.id ?? null);
  const current = connections.find((x) => x.id === editing) ?? null;
  const [form, setForm] = useState<Form>(() => (current ? formOf(c, current) : emptyForm(c)));
  const select = (v: ConnectionView | null) => {
    setEditing(v?.id ?? null);
    setForm(v ? formOf(c, v) : emptyForm(c));
    test.reset();
    save.reset();
  };
  const input = inputOf(c, form, current?.id);
  const test = useMutation({ mutationFn: () => testConnection(c.id, input) });
  const save = useMutation({
    mutationFn: () => (current ? updateConnection(c.id, current.id, input) : createConnection(c.id, input)),
    onSuccess: (v) => {
      setEditing(v.id);
      setForm((f) => ({ ...f, name: v.name, secrets: {} }));
      return refresh();
    },
  });
  const remove = useMutation({ mutationFn: (id: string) => deleteConnection(c.id, id), onSuccess: () => (select(null), refresh()) });
  const sync = useMutation({ mutationFn: (id: string) => syncConnection(c.id, id), onSuccess: refresh });
  const dashboards = useDashboardLinks(c);
  const set = (patch: Partial<Form>) => {
    setForm((f) => ({ ...f, ...patch }));
    test.reset();
  };
  const missing = c.config.filter((f) => f.required && (f.secret ? !form.secrets[f.key]?.trim() && !current?.secrets.includes(f.key) : f.default === null && !String(form.config[f.key] ?? '').trim()));
  const ready = missing.length === 0;
  const logsQuery = `source:${c.source}`;
  return (
    <div className="ci-setup">
      <Step n={1} title={`Prepare ${c.vendor}`} open onToggle={() => {}} done={connections.length > 0}>
        {c.setup ? (
          <div className="muted">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{c.setup}</ReactMarkdown>
          </div>
        ) : (
          <p className="muted">Create an API credential for ObserveCraft in {c.vendor}, read-only where it can be.</p>
        )}
        {c.links.length > 0 && (
          <ul className="ci-otlp">
            {c.links.map((l) => (
              <li key={l.label}>
                <a href={fill(l.url, { config: form.config })} target="_blank" rel="noreferrer">
                  {l.label}
                </a>
              </li>
            ))}
          </ul>
        )}
      </Step>

      <Step n={2} title={current ? `Connection: ${current.name}` : 'Connect an account'} open onToggle={() => {}} done={!!current}>
        {connections.length > 0 && (
          <div className="rt-create-actions">
            {connections.map((v) => (
              <button key={v.id} type="button" className={`btn${v.id === editing ? ' btn-primary' : ''}`} onClick={() => select(v)}>
                {v.name}
              </button>
            ))}
            <button type="button" className={`btn${!current ? ' btn-primary' : ''}`} onClick={() => select(null)} disabled={!isAdmin}>
              <Plus size={14} /> Add a connection
            </button>
          </div>
        )}
        <label className="rt-field">
          <span>Name</span>
          <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="The account's host by default" disabled={!isAdmin} />
        </label>
        {c.config.map((f) => (
          <Field
            key={f.key}
            f={f}
            value={f.secret ? (form.secrets[f.key] ?? '') : (form.config[f.key] ?? '')}
            saved={!!f.secret && !!current?.secrets.includes(f.key)}
            disabled={!isAdmin}
            onChange={(v) => (f.secret ? set({ secrets: { ...form.secrets, [f.key]: String(v) } }) : set({ config: { ...form.config, [f.key]: v } }))}
          />
        ))}
        {c.streams.length > 1 && (
          <fieldset className="rt-field">
            <span>Collect</span>
            {c.streams.map((s) => (
              <label key={s.id} className="muted">
                <input
                  type="checkbox"
                  checked={form.streams.includes(s.id)}
                  disabled={!isAdmin}
                  onChange={(e) => set({ streams: e.target.checked ? [...form.streams, s.id] : form.streams.filter((x) => x !== s.id) })}
                />{' '}
                <b>{s.name}</b> — {s.description}
              </label>
            ))}
          </fieldset>
        )}
      </Step>

      <Step n={3} title="Test and save" open onToggle={() => {}} done={!!current}>
        {!isAdmin && <p className="muted">Only organization admins can change {c.name} connections.</p>}
        {!ready && <p className="muted">Still needed: {missing.map((f) => f.label).join(', ')}.</p>}
        {test.data && (
          <div className={test.data.ok ? 'rt-note' : 'rt-warning'}>
            {test.data.ok ? <CircleCheck size={16} /> : <AlertTriangle size={16} />}
            <div>{test.data.message}</div>
          </div>
        )}
        {[test.error, save.error, remove.error, sync.error].filter(Boolean).map((e, i) => (
          <p key={i} className="form-error">
            {(e as Error).message}
          </p>
        ))}
        {save.isSuccess && !save.isPending && <p className="muted">Saved. The first sync starts within a minute.</p>}
        <div className="rt-create-actions">
          {c.testable && (
            <button type="button" className="btn" disabled={!isAdmin || !ready || test.isPending} onClick={() => test.mutate()}>
              {test.isPending ? 'Testing…' : 'Test connection'}
            </button>
          )}
          <button type="button" className="btn btn-primary" disabled={!isAdmin || !ready || save.isPending} onClick={() => save.mutate()}>
            {save.isPending ? 'Saving…' : current ? 'Save changes' : 'Save and start syncing'}
          </button>
          {current && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={!isAdmin || remove.isPending}
              onClick={() => window.confirm(`Remove the connection ${current.name}? Data already imported stays.`) && remove.mutate(current.id)}
            >
              Remove
            </button>
          )}
        </div>
      </Step>

      <Step n={4} title="Wait for the first data" open onToggle={() => {}} done={!!current?.streams.some((s) => s.lastSuccessAt)}>
        {!current ? (
          <p className="muted">Once a connection is saved, ObserveCraft reads {c.vendor}&apos;s API on each stream&apos;s schedule.</p>
        ) : (
          <>
            <table className="ci-sync-table">
              <thead>
                <tr>
                  <th>Stream</th>
                  <th>Last run</th>
                  <th>Read up to</th>
                  <th>Records</th>
                </tr>
              </thead>
              <tbody>
                {current.streams.map((s) => (
                  <tr key={s.id} className={s.enabled ? '' : 'muted'}>
                    <td>{s.name}</td>
                    <td>{!s.enabled ? 'Off' : s.lastSuccessAt ? timeAgo(s.lastSuccessAt) : 'Waiting for the first run…'}</td>
                    <td>{s.collectedUntil ? new Date(s.collectedUntil).toLocaleString() : '—'}</td>
                    <td>{s.records.toLocaleString()}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {current.streams
              .filter((s) => s.error)
              .map((s) => (
                <div key={s.id} className="rt-warning">
                  <AlertTriangle size={16} />
                  <div>
                    <b>{s.name}:</b> {s.error!.message} <span className="muted">({timeAgo(s.error!.at)})</span>
                  </div>
                </div>
              ))}
            <div className="rt-create-actions">
              <button type="button" className="btn" disabled={!isAdmin || sync.isPending} onClick={() => sync.mutate(current.id)}>
                <RefreshCw size={14} /> {sync.isPending ? 'Syncing…' : 'Sync now'}
              </button>
              {c.streams.some((s) => s.logs) && (
                <Link className="btn" to={`/logs?query=${encodeURIComponent(logsQuery)}`}>
                  View logs
                </Link>
              )}
              {dashboards.map((d) => (
                <Link key={d.id} className="btn btn-primary" to={`/dashboards/${d.id}`}>
                  {d.title}
                </Link>
              ))}
            </div>
          </>
        )}
      </Step>
    </div>
  );
}

/** A signed push's signing secret, saved as the org's connection of the package: the intake checks requests with it. */
function SigningSecret({ c, n, connections }: { c: ConnectorEntry; n: number; connections: ConnectionView[] }) {
  const qc = useQueryClient();
  const { isAdmin } = useAccess();
  const field = c.config.find((f) => f.key === c.push?.signatureSecret);
  const current = connections[0] ?? null;
  const [value, setValue] = useState('');
  const save = useMutation({
    mutationFn: () => {
      const input = { ...inputOf(c, current ? formOf(c, current) : emptyForm(c), current?.id), secrets: { [field!.key]: value.trim() } } as ConnectionInput;
      return current ? updateConnection(c.id, current.id, input) : createConnection(c.id, input);
    },
    onSuccess: () => (setValue(''), Promise.all([qc.invalidateQueries({ queryKey: ['connectors', c.id] }), qc.invalidateQueries({ queryKey: ['integrations'] })])),
  });
  if (!field) return null;
  const saved = !!current?.secrets.includes(field.key);
  return (
    <Step n={n} title="Save the signing secret" open onToggle={() => {}} done={saved}>
      <p className="muted">
        {saved
          ? `Requests without a valid ${c.push!.signatureHeader} signature are refused. Enter a new secret to replace it.`
          : `Until it is saved, the ingest API key alone admits ${c.vendor}'s requests.`}
      </p>
      <Field f={field} value={value} saved={saved} disabled={!isAdmin} onChange={(v) => (setValue(String(v)), save.reset())} />
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
      {save.isSuccess && <p className="muted">Saved. The intake checks signatures with it within a minute.</p>}
      <div className="rt-create-actions">
        <button type="button" className="btn btn-primary" disabled={!isAdmin || !value.trim() || save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? 'Saving…' : 'Save secret'}
        </button>
        {current && (
          <button
            type="button"
            className="btn btn-danger"
            disabled={!isAdmin}
            onClick={() =>
              window.confirm('Remove the signing secret? Requests are then admitted by the API key alone.') &&
              deleteConnection(c.id, current.id).then(() => qc.invalidateQueries({ queryKey: ['connectors', c.id] }))
            }
          >
            Remove
          </button>
        )}
      </div>
    </Step>
  );
}

/** Where a push of `kind` sends by default (a package's `push.url` replaces it). */
function defaultPushUrl(kind: string, webhook: string): string {
  if (kind === 'otlp') return '{{ intake }}';
  if (kind === 'webhook') return `{{ intake }}/intake/webhook/${webhook}?api_key={{ api_key }}`;
  if (kind === 'splunk_hec') return '{{ intake }}/services/collector?ddsource={{ source }}';
  return '{{ intake }}/api/v2/logs?ddsource={{ source }}&dd-api-key={{ api_key }}';
}

function Push({ c, connections }: { c: ConnectorEntry; connections: ConnectionView[] }) {
  const { isAdmin } = useAccess();
  const p = c.push!;
  const vars = { intake: intakeUrl(), source: c.source, api_key: '<INGEST_API_KEY>' };
  const url = fill(p.url ?? defaultPushUrl(p.kind, p.webhook ?? c.id), vars);
  const [form, setForm] = useState<Form>(() => emptyForm(c));
  const create = useMutation({ mutationFn: () => createPush(c.id, { ...inputOf(c, form), pushUrl: url } as ConnectionInput) });
  const headers = useMemo(
    () =>
      Object.entries(p.headers)
        .map(([k, v]) => `${k}: ${fill(v, vars)}`)
        .join('\n'),
    [p.headers, vars],
  );
  return (
    <div className="ci-setup">
      <Step n={1} title={`Point ${c.vendor} at ObserveCraft`} open onToggle={() => {}}>
        <div className="muted">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{fill(p.instructions, vars)}</ReactMarkdown>
        </div>
        <CodeBlock code={url} />
        {headers && <CodeBlock code={headers} />}
        {p.kind === 'splunk_hec' && <CodeBlock code={`Authorization: Splunk ${vars.api_key}`} />}
        {p.endpoints.map((e) => (
          <div key={e.label}>
            <h4>{e.label}</h4>
            {e.instructions && (
              <div className="muted">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{fill(e.instructions, vars)}</ReactMarkdown>
              </div>
            )}
            <CodeBlock code={fill(e.url ?? defaultPushUrl(e.kind, p.webhook ?? c.id), vars)} />
            {Object.keys(e.headers).length > 0 && (
              <CodeBlock
                code={Object.entries(e.headers)
                  .map(([k, v]) => `${k}: ${fill(v, vars)}`)
                  .join('\n')}
              />
            )}
          </div>
        ))}
        {p.signatureHeader && (
          <p className="muted">
            ObserveCraft checks the {p.signatureHeader} signature of every request with the signing secret {c.vendor} gives you.
          </p>
        )}
      </Step>
      {p.signatureSecret && <SigningSecret c={c} n={2} connections={connections} />}
      {p.canCreate && (
        <Step n={p.signatureSecret ? 3 : 2} title={`Or create it from here`} open onToggle={() => {}}>
          {c.config
            .filter((f) => f.key !== p.signatureSecret)
            .map((f) => (
              <Field
                key={f.key}
                f={f}
                value={f.secret ? (form.secrets[f.key] ?? '') : (form.config[f.key] ?? '')}
                saved={false}
                disabled={!isAdmin}
                onChange={(v) => setForm((x) => (f.secret ? { ...x, secrets: { ...x.secrets, [f.key]: String(v) } } : { ...x, config: { ...x.config, [f.key]: v } }))}
              />
            ))}
          {create.data && (
            <div className={create.data.ok ? 'rt-note' : 'rt-warning'}>
              {create.data.ok ? <CircleCheck size={16} /> : <AlertTriangle size={16} />}
              <div>{create.data.message}</div>
            </div>
          )}
          {create.error && <p className="form-error">{(create.error as Error).message}</p>}
          <div className="rt-create-actions">
            <button type="button" className="btn btn-primary" disabled={!isAdmin || create.isPending} onClick={() => create.mutate()}>
              {create.isPending ? 'Creating…' : `Create on ${c.vendor}`}
            </button>
          </div>
        </Step>
      )}
    </div>
  );
}

function DataCollected({ c }: { c: ConnectorEntry }) {
  const dashboards = useDashboardLinks(c);
  return (
    <div className="ci-setup">
      {c.dashboards.length + dashboards.length > 0 && (
        <>
          <h3>Dashboards</h3>
          <ul className="ci-otlp">
            {dashboards.length
              ? dashboards.map((d) => (
                  <li key={d.id}>
                    <Link to={`/dashboards/${d.id}`}>{d.title}</Link>
                  </li>
                ))
              : c.dashboards.map((t) => (
                  <li key={t}>
                    {t} <span className="muted">— listed in Dashboards once {c.name} is connected</span>
                  </li>
                ))}
          </ul>
        </>
      )}
      {c.streams.map((s) => (
        <section key={s.id}>
          <h3>{s.name}</h3>
          <p className="muted">
            {s.description} Every {s.intervalSecs >= 3600 ? `${s.intervalSecs / 3600} h` : `${Math.round(s.intervalSecs / 60)} min`}.
          </p>
          {s.logs && (
            <p className="muted">
              Logs with <Link to={`/logs?query=${encodeURIComponent(`source:${c.source}`)}`}>source:{c.source}</Link>
              {c.pipeline ? `, processed by the ${c.pipeline} pipeline` : ''}.
            </p>
          )}
          {s.events && <p className="muted">Events in Event Management.</p>}
          {s.costs && <p className="muted">Costs in Cloud Cost Management.</p>}
          {s.metrics.length > 0 && (
            <dl className="ci-sync">
              {s.metrics.map((m) => (
                <div key={m.name} style={{ display: 'contents' }}>
                  <dt>
                    <code>{m.name}</code>
                  </dt>
                  <dd>
                    {m.description ?? m.type}
                    {m.unit ? ` (${m.unit})` : ''}
                    {m.tags.length > 0 && <span className="muted"> — tags {m.tags.join(', ')}</span>}
                  </dd>
                </div>
              ))}
            </dl>
          )}
        </section>
      ))}
      {c.monitors.length > 0 && (
        <>
          <h3>Recommended monitors</h3>
          <ul className="ci-otlp">
            {c.monitors.map((m) => (
              <li key={m}>{m}</li>
            ))}
          </ul>
        </>
      )}
      <p className="muted">
        Connector package <code>{c.id}</code> {c.version} by {c.publisher}
        {c.thirdParty ? ' (third-party)' : ''}. <Link to={connectorPage(c)}>Configuration</Link>
      </p>
    </div>
  );
}

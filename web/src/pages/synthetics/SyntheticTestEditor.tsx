import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { AlertTriangle, Check, ChevronRight, MapPin, Plus, Radar, Send, Sparkles, Trash2, X } from 'lucide-react';
import type { SyntheticAssertion, SyntheticAssertionResult, SyntheticRequest, SyntheticTest, SyntheticTestInput, SyntheticTestType } from '../../api/synthetics/client';
import {
  ASSERTION_LABEL,
  ASSERTION_TYPES,
  FREQUENCIES,
  METHODS,
  OPERATORS,
  OPERATOR_LABEL,
  RECORD_TYPES,
  SNIPPETS,
  SNIPPET_CATEGORIES,
  SOON_TYPES,
  TEST_TYPES,
  TYPE_LABEL,
  assertionUnit,
  draftOf,
  newAssertion,
  newDraft,
  propertyKind,
  requestProblem,
  withType,
  type DraftDefaults,
  type TestDraft,
} from '../../api/synthetics/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Step } from '../monitors/MonitorEditor';
import { PRIORITY_LABEL, RecipientIcon, recipientLabel } from '../monitors/common';
import { messageRecipients } from '../monitors/message';
import { RunView } from './common';
import { useSaveSyntheticTest, useSyntheticLocations, useSyntheticSettings, useSyntheticTest, useTrySyntheticRequest } from './queries';
import { fullSettings } from './SyntheticsSettings';
import '../monitors/monitors.css';
import './synthetics.css';

const BODY_TYPES = ['', 'application/json', 'text/plain', 'application/x-www-form-urlencoded', 'application/xml'];
const list = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

/** New API Test / Edit: a numbered form of request, assertions, location, retries, schedule, downtimes and monitor. */
export function SyntheticTestEditor() {
  const { id } = useParams();
  const q = useSyntheticTest(id);
  // New tests start from Settings → Default Settings (the built-in defaults if they can't be read).
  const settings = useSyntheticSettings();
  if (id && q.isPending) return <Loading label="Loading test…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!id && settings.isPending) return <Loading label="Loading default settings…" />;
  return <EditorForm key={id ?? 'new'} test={q.data} defaults={fullSettings(settings.data).defaults} />;
}

function EditorForm({ test, defaults }: { test?: SyntheticTest; defaults: DraftDefaults }) {
  const navigate = useNavigate();
  const [d, setD] = useState<TestDraft>(() => (test ? draftOf(test) : newDraft('http', defaults)));
  const [tags, setTags] = useState(() => d.tags.join(', '));
  const [advanced, setAdvanced] = useState(() => !!test && (test.request.headers.length > 0 || !!test.request.body || !!test.request.basicAuth));
  const send = useTrySyntheticRequest();
  const save = useSaveSyntheticTest(test?.id);
  const locations = useSyntheticLocations();
  const location = locations.data?.[0];
  const set = (p: Partial<TestDraft>) => setD((x) => ({ ...x, ...p }));
  const setReq = (p: Partial<SyntheticRequest>) => setD((x) => ({ ...x, request: { ...x.request, ...p } }));

  const problem = requestProblem(d.type, d.request);
  const requestOk = !problem && !!d.name.trim();
  const assertionsOk = d.assertions.every((a) => a.target.trim() !== '' && (propertyKind(a.type) === null || (a.property ?? '').trim() !== ''));
  const missing = [!requestOk && 2, !assertionsOk && 3].filter(Boolean) as number[];
  const input: SyntheticTestInput = { ...d, tags: list(tags) };
  const sent = send.data;

  const doSend = () =>
    send.mutate(
      { type: d.type, request: d.request, assertions: d.assertions },
      { onSuccess: (r) => d.assertions.length === 0 && r.suggestedAssertions.length > 0 && set({ assertions: r.suggestedAssertions }) },
    );
  const submit = () => save.mutate(input, { onSuccess: (t) => navigate(`/synthetics/tests/${t.id}`) });
  const setAssertion = (i: number, a: SyntheticAssertion) => set({ assertions: d.assertions.map((x, j) => (j === i ? a : x)) });
  const resultFor = (a: SyntheticAssertion): SyntheticAssertionResult | undefined =>
    sent?.result.assertions.find((r) => r.assertion.type === a.type && r.assertion.operator === a.operator && (r.assertion.property ?? '') === (a.property ?? '') && r.assertion.target === a.target);

  return (
    <div className="mon-editor">
      <header className="mon-crumbs">
        <Link to="/synthetics/tests" className="mon-crumb-root">
          <Radar size={22} /> Synthetic Tests
        </Link>
        <ChevronRight size={18} className="muted" />
        {test ? (
          <>
            <Link to={`/synthetics/tests/${test.id}`} className="mon-crumb-link">
              {test.name}
            </Link>
            <ChevronRight size={18} className="muted" />
            <span>Edit</span>
          </>
        ) : (
          <span>New API Test</span>
        )}
      </header>
      <div className="mon-editor-scroll">
        <div className="syn-editor-grid">
          <div className="mon-steps">
            <Step n={1} title="Request Type" done>
              <div className="syn-types" role="radiogroup" aria-label="Request type">
                {TEST_TYPES.map((t) => (
                  <button key={t} role="radio" aria-checked={d.type === t} className={`syn-type-btn${d.type === t ? ' active' : ''}`} onClick={() => setD((x) => withType(x, t))}>
                    {TYPE_LABEL[t]}
                  </button>
                ))}
                {SOON_TYPES.map((t) => (
                  <button key={t} className="syn-type-btn" disabled title="Not available yet">
                    {t} <span className="syn-soon">soon</span>
                  </button>
                ))}
              </div>
            </Step>
            <Step n={2} title="Define Request" done={requestOk}>
              <RequestFields type={d.type} r={d.request} setReq={setReq} onSend={doSend} sending={send.isPending} canSend={!problem} />
              {problem && d.request.url !== 'https://' && <p className="syn-hint warn">{problem}</p>}
              <p className="syn-hint muted">
                Write <code>{'{{ NAME }}'}</code> to use a <Link to="/synthetics/settings/variables">global variable</Link> in the URL, host, headers, body or credentials.
              </p>
              <button className="link-btn syn-advanced-toggle" onClick={() => setAdvanced((a) => !a)}>
                {advanced ? '▾' : '▸'} Advanced Options
              </button>
              {advanced && <AdvancedOptions type={d.type} r={d.request} setReq={setReq} />}
              {send.isError && <p className="form-error">{(send.error as Error).message}</p>}
              {sent && (
                <div className="syn-send-result">
                  <div className="syn-send-head">
                    <span>
                      Response from <b>{sent.location.name}</b>
                    </span>
                    <button className="icon-plain" title="Close" onClick={() => send.reset()}>
                      <X size={16} />
                    </button>
                  </div>
                  <RunView run={sent.result} type={d.type} />
                </div>
              )}
              <div className="syn-name-grid">
                <label className="form-label syn-span-2">
                  Name*
                  <input className="syn-input" value={d.name} onChange={(e) => set({ name: e.target.value })} placeholder="e.g. Checkout API health" />
                </label>
                <label className="form-label">
                  Environment
                  <input className="syn-input" value={d.env} onChange={(e) => set({ env: e.target.value })} placeholder="prod" />
                </label>
                <label className="form-label">
                  Team
                  <input className="syn-input" value={d.team} onChange={(e) => set({ team: e.target.value })} placeholder="checkout" />
                </label>
                <label className="form-label syn-span-2">
                  Additional tags
                  <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="service:order-service, tier:critical" />
                </label>
              </div>
            </Step>
            <Step n={3} title="Define Assertions" done={assertionsOk && d.assertions.length > 0}>
              {d.assertions.length === 0 && (
                <p className="muted">
                  No assertions: a run passes when the {d.type === 'dns' ? 'lookup returns records' : d.type === 'http' ? 'server answers' : 'connection succeeds'}. Send the request to get
                  suggestions.
                </p>
              )}
              {d.assertions.map((a, i) => (
                <AssertionRow key={i} type={d.type} a={a} result={resultFor(a)} onChange={(x) => setAssertion(i, x)} onRemove={() => set({ assertions: d.assertions.filter((_, j) => j !== i) })} />
              ))}
              <div className="syn-row">
                <button className="btn" onClick={() => set({ assertions: [...d.assertions, newAssertion(ASSERTION_TYPES[d.type][0])] })} disabled={d.assertions.length >= 20}>
                  <Plus size={14} /> New Assertion
                </button>
                {sent && sent.suggestedAssertions.length > 0 && (
                  <button className="btn" onClick={() => set({ assertions: sent.suggestedAssertions })}>
                    <Sparkles size={14} /> Use the {sent.suggestedAssertions.length} suggested assertions
                  </button>
                )}
              </div>
            </Step>
            <Step n={4} title="Select Locations" done>
              <label className="syn-location">
                <input type="checkbox" className="checkbox" checked disabled />
                <MapPin size={16} />
                <span>
                  <b>{location?.name ?? 'ObserveCraft server'}</b>
                  <span className="muted">
                    Tests run from the ObserveCraft server itself.{' '}
                    {location?.privateTargets ? 'This server may reach private addresses.' : 'Private, loopback and link-local addresses are refused (OC_SYNTHETICS_ALLOW_PRIVATE).'}
                  </span>
                </span>
              </label>
            </Step>
            <Step n={5} title="Specify Test Retry Conditions" done>
              <div className="mon-eval-row">
                <label>
                  Retry test
                  <select className="syn-select" value={d.retry.count} onChange={(e) => set({ retry: { ...d.retry, count: Number(e.target.value) } })} aria-label="Retries">
                    {[0, 1, 2, 3].map((n) => (
                      <option key={n} value={n}>
                        {n}
                      </option>
                    ))}
                  </select>
                  times after
                </label>
                <input
                  className="mon-num"
                  value={d.retry.intervalMs}
                  onChange={(e) => set({ retry: { ...d.retry, intervalMs: Math.max(0, Math.min(60_000, Number(e.target.value) || 0)) } })}
                  inputMode="numeric"
                  aria-label="Retry interval"
                  disabled={d.retry.count === 0}
                />
                <label>ms in case of failure</label>
              </div>
            </Step>
            <Step n={6} title="Define Scheduling and Alert Conditions" done>
              <div className="syn-sched">
                <span>Run the test every</span>
                <span className="btn-group" role="group" aria-label="Frequency">
                  {FREQUENCIES.map((f) => (
                    <button key={f.seconds} className={`btn${d.frequencySeconds === f.seconds ? ' btn-active' : ''}`} onClick={() => set({ frequencySeconds: f.seconds })}>
                      {f.label}
                    </button>
                  ))}
                </span>
              </div>
              <div className="mon-eval-row">
                <label>An alert is triggered if the test fails for</label>
                <input
                  className="mon-num"
                  value={d.minFailureDurationMinutes}
                  onChange={(e) => set({ minFailureDurationMinutes: Math.max(0, Math.min(120, Number(e.target.value) || 0)) })}
                  inputMode="numeric"
                  aria-label="Minutes failing"
                />
                <label>
                  minutes from
                  <select className="syn-select" value={d.minLocationFailed} disabled aria-label="Locations failing">
                    <option value={1}>1</option>
                  </select>
                  of 1 location
                </label>
              </div>
              <label className="mon-check syn-pause">
                <input type="checkbox" className="checkbox" checked={d.paused} onChange={() => set({ paused: !d.paused })} />
                <span className="mon-check-label">Paused: no runs and no alerts until resumed</span>
              </label>
            </Step>
            <Step n={7} title="Downtimes" done>
              <p className="muted syn-text">
                Downtimes from <Link to="/monitors/downtimes">Monitors → Downtimes</Link> silence this test's notifications when they target its monitor or all of its tags. The test keeps running and
                recording results during them.
              </p>
            </Step>
            <Step n={8} title="Configure the Monitor" done={!!d.message.trim()} last>
              <p className="muted syn-text">
                The test alerts through its own monitor, <b>[Synthetics] {d.name || 'name'}</b>. Mention <code>@webhook-…</code>, <code>@slack-…</code> or email addresses to notify;{' '}
                <code>{'{{#is_alert}}'}</code> and <code>{'{{#is_recovery}}'}</code> blocks pick the text per transition. What failed is added to alert notifications.
              </p>
              <textarea className="syn-textarea" rows={5} value={d.message} onChange={(e) => set({ message: e.target.value })} aria-label="Notification message" />
              <div className="syn-row">
                {messageRecipients(d.message).map((r) => (
                  <span key={r} className="syn-chip">
                    <RecipientIcon r={r} size={13} /> {recipientLabel(r)}
                  </span>
                ))}
                <label className="syn-priority">
                  Priority
                  <select className="syn-select" value={d.priority ?? 0} onChange={(e) => set({ priority: Number(e.target.value) || null })}>
                    {['0', '1', '2', '3', '4', '5'].map((p) => (
                      <option key={p} value={p}>
                        {PRIORITY_LABEL[p]}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </Step>
          </div>
          <aside className="syn-snippets">
            <div className="syn-snippets-tab">Snippets</div>
            <div className="syn-snippets-intro">
              <Sparkles size={18} />
              <span>Start from pre-filled settings for common checks.</span>
            </div>
            {SNIPPET_CATEGORIES.map((c) => {
              const items = SNIPPETS.filter((s) => s.category === c);
              return (
                <section key={c} className="syn-snippet-group">
                  <h4>
                    {c} <span className="muted">{items.length} snippets</span>
                  </h4>
                  {items.map((s) => (
                    <button key={s.id} className="syn-snippet" onClick={() => setD((x) => s.apply(x))}>
                      <b>{s.title}</b>
                      <span className="muted">{s.description}</span>
                    </button>
                  ))}
                </section>
              );
            })}
          </aside>
        </div>
      </div>
      <footer className="mon-editor-foot">
        {missing.length ? (
          <span className="mon-foot-warn">
            <AlertTriangle size={20} /> Complete step{missing.length > 1 ? 's' : ''} {missing.join(' and ')}
          </span>
        ) : (
          <span className="mon-foot-ok">
            <Check size={18} /> Ready to {test ? 'save' : 'create'}
          </span>
        )}
        {save.isError && <span className="form-error">{(save.error as Error).message}</span>}
        <span className="mon-foot-actions">
          <button className="btn" onClick={() => navigate(test ? `/synthetics/tests/${test.id}` : '/synthetics/tests')}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={missing.length > 0 || save.isPending} onClick={submit}>
            Save Test
          </button>
        </span>
      </footer>
    </div>
  );
}

function RequestFields({
  type,
  r,
  setReq,
  onSend,
  sending,
  canSend,
}: {
  type: SyntheticTestType;
  r: SyntheticRequest;
  setReq: (p: Partial<SyntheticRequest>) => void;
  onSend: () => void;
  sending: boolean;
  canSend: boolean;
}) {
  const port = (v: string) => (v.trim() === '' ? null : Math.max(0, Math.min(65_535, Number(v.replace(/\D/g, '')) || 0)));
  const sendButton = (
    <button className="btn btn-primary" onClick={onSend} disabled={!canSend || sending}>
      <Send size={14} /> {sending ? 'Sending…' : 'Send'}
    </button>
  );
  if (type === 'http')
    return (
      <div className="syn-request-panel">
        <span className="syn-request-label">
          URL <span className="syn-required">*</span>
        </span>
        <div className="syn-request">
          <select className="syn-select syn-method" value={r.method} onChange={(e) => setReq({ method: e.target.value })} aria-label="Method">
            {METHODS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
          <input className="syn-input syn-grow mono" value={r.url} onChange={(e) => setReq({ url: e.target.value })} placeholder="https://shop.example.com/health" aria-label="URL" />
          {sendButton}
        </div>
      </div>
    );
  return (
    <div className="syn-request-panel">
      <span className="syn-request-label">
        {type === 'dns' ? 'Domain' : 'Host'} <span className="syn-required">*</span>
      </span>
      <div className="syn-request">
        <input
          className="syn-input syn-grow mono"
          value={r.host}
          onChange={(e) => setReq({ host: e.target.value })}
          placeholder={type === 'dns' ? 'shop.example.com' : 'host name or IP address'}
          aria-label={type === 'dns' ? 'Domain' : 'Host'}
        />
        {type === 'dns' ? (
          <>
            <input className="syn-input syn-server mono" value={r.dnsServer} onChange={(e) => setReq({ dnsServer: e.target.value })} placeholder="DNS server (optional)" aria-label="DNS server" />
            <input className="syn-input syn-port" value={r.dnsServerPort ?? ''} onChange={(e) => setReq({ dnsServerPort: port(e.target.value) })} placeholder="53" aria-label="DNS server port" />
          </>
        ) : (
          <input className="syn-input syn-port" value={r.port ?? ''} onChange={(e) => setReq({ port: port(e.target.value) })} placeholder={type === 'ssl' ? '443' : 'Port'} aria-label="Port" />
        )}
        {sendButton}
      </div>
    </div>
  );
}

function AdvancedOptions({ type, r, setReq }: { type: SyntheticTestType; r: SyntheticRequest; setReq: (p: Partial<SyntheticRequest>) => void }) {
  const setHeader = (i: number, p: Partial<{ name: string; value: string }>) => setReq({ headers: r.headers.map((h, j) => (j === i ? { ...h, ...p } : h)) });
  return (
    <div className="syn-advanced">
      {type === 'http' && (
        <>
          <h4>Request headers</h4>
          {r.headers.map((h, i) => (
            <div key={i} className="syn-row">
              <input className="syn-input mono" value={h.name} onChange={(e) => setHeader(i, { name: e.target.value })} placeholder="Name" aria-label="Header name" />
              <input className="syn-input syn-grow mono" value={h.value} onChange={(e) => setHeader(i, { value: e.target.value })} placeholder="Value" aria-label="Header value" />
              <button className="icon-plain" title="Remove header" onClick={() => setReq({ headers: r.headers.filter((_, j) => j !== i) })}>
                <Trash2 size={15} />
              </button>
            </div>
          ))}
          <button className="link-btn" onClick={() => setReq({ headers: [...r.headers, { name: '', value: '' }] })}>
            <Plus size={14} /> Add header
          </button>
          <h4>Authentication</h4>
          <label className="mon-check">
            <input type="checkbox" className="checkbox" checked={!!r.basicAuth} onChange={() => setReq({ basicAuth: r.basicAuth ? null : { username: '', password: '' } })} />
            <span className="mon-check-label">Basic authentication</span>
          </label>
          {r.basicAuth && (
            <div className="syn-row">
              <input
                className="syn-input"
                value={r.basicAuth.username}
                onChange={(e) => setReq({ basicAuth: { ...r.basicAuth!, username: e.target.value } })}
                placeholder="Username"
                aria-label="Username"
              />
              <input
                className="syn-input"
                type="password"
                value={r.basicAuth.password}
                onChange={(e) => setReq({ basicAuth: { ...r.basicAuth!, password: e.target.value } })}
                placeholder="Password"
                aria-label="Password"
                autoComplete="new-password"
              />
            </div>
          )}
          <h4>Request body</h4>
          <select className="syn-select" value={r.bodyType} onChange={(e) => setReq({ bodyType: e.target.value })} aria-label="Body type">
            {BODY_TYPES.map((t) => (
              <option key={t} value={t}>
                {t || 'No content type'}
              </option>
            ))}
          </select>
          <textarea className="syn-textarea mono" rows={4} value={r.body} onChange={(e) => setReq({ body: e.target.value })} placeholder='{"query": "running shoes"}' aria-label="Body" />
        </>
      )}
      <h4>Options</h4>
      <div className="mon-eval-row">
        <label>Timeout</label>
        <input
          className="mon-num"
          value={r.timeoutSeconds}
          onChange={(e) => setReq({ timeoutSeconds: Math.max(1, Math.min(60, Number(e.target.value) || 1)) })}
          inputMode="numeric"
          aria-label="Timeout (seconds)"
        />
        <label>seconds (at most 60)</label>
      </div>
      {type === 'http' && (
        <label className="mon-check">
          <input type="checkbox" className="checkbox" checked={r.followRedirects} onChange={() => setReq({ followRedirects: !r.followRedirects })} />
          <span className="mon-check-label">Follow redirects (up to 10, each one checked again)</span>
        </label>
      )}
      {(type === 'http' || type === 'ssl') && (
        <label className="mon-check">
          <input type="checkbox" className="checkbox" checked={r.acceptSelfSigned} onChange={() => setReq({ acceptSelfSigned: !r.acceptSelfSigned })} />
          <span className="mon-check-label">Accept self-signed and untrusted certificates</span>
        </label>
      )}
    </div>
  );
}

function AssertionRow({
  type,
  a,
  result,
  onChange,
  onRemove,
}: {
  type: SyntheticTestType;
  a: SyntheticAssertion;
  result?: SyntheticAssertionResult;
  onChange: (a: SyntheticAssertion) => void;
  onRemove: () => void;
}) {
  const kind = propertyKind(a.type);
  const unit = assertionUnit(a.type);
  return (
    <div className="syn-assertion">
      <select className="syn-select" value={a.type} onChange={(e) => onChange(newAssertion(e.target.value as SyntheticAssertion['type']))} aria-label="Assertion type">
        {ASSERTION_TYPES[type].map((t) => (
          <option key={t} value={t}>
            {ASSERTION_LABEL[t]}
          </option>
        ))}
      </select>
      {kind === 'record' ? (
        <select className="syn-select" value={a.property} onChange={(e) => onChange({ ...a, property: e.target.value })} aria-label="Record type">
          {RECORD_TYPES.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>
      ) : (
        kind && (
          <input
            className="syn-input mono"
            value={a.property}
            onChange={(e) => onChange({ ...a, property: e.target.value })}
            placeholder={kind === 'header' ? 'header name' : '$.path.to.field'}
            aria-label={kind === 'header' ? 'Header name' : 'JSON path'}
          />
        )
      )}
      <select className="syn-select" value={a.operator} onChange={(e) => onChange({ ...a, operator: e.target.value as SyntheticAssertion['operator'] })} aria-label="Operator">
        {OPERATORS[a.type].map((o) => (
          <option key={o} value={o}>
            {OPERATOR_LABEL[o]}
          </option>
        ))}
      </select>
      <input className="syn-input syn-grow mono" value={a.target} onChange={(e) => onChange({ ...a, target: e.target.value })} aria-label="Target" />
      {unit && <span className="muted">{unit}</span>}
      {result && (
        <span className={`syn-assert-result ${result.passed ? 'pass' : 'fail'}`} title={result.error ?? (result.actual != null ? `Got ${result.actual}` : 'Nothing found')}>
          {result.passed ? <Check size={14} /> : <X size={14} />} {result.actual ?? '—'}
        </span>
      )}
      <button className="icon-plain" title="Remove assertion" onClick={onRemove}>
        <Trash2 size={15} />
      </button>
    </div>
  );
}

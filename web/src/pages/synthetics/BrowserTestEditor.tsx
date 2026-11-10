import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { AlertTriangle, ArrowDown, ArrowUp, Check, ChevronRight, Copy, MonitorSmartphone, Play, Plus, Radar, Trash2, X } from 'lucide-react';
import type { BrowserElement, BrowserStep, BrowserStepType, SyntheticTest, SyntheticTestInput, SyntheticTryResult } from '../../api/synthetics/client';
import {
  CHECKS,
  CHECK_LABEL,
  DEVICES,
  KEYS,
  MIN_BROWSER_FREQUENCY,
  MODIFIERS,
  STEP_GROUPS,
  STEP_LABEL,
  needsElement,
  newStep,
  stepName,
  stepProblem,
  stepValue,
  usesCheck,
} from '../../api/synthetics/browserSteps';
import { draftOf, newDraft, requestProblem, type DraftDefaults, type TestDraft } from '../../api/synthetics/logic';
import { Menu } from '../../components/ui/Dropdown';
import { Step } from '../monitors/MonitorEditor';
import { BrowserRunView } from './BrowserRun';
import { CommonSteps } from './editorSections';
import { useSaveSyntheticTest, useSyntheticLocations, useTrySyntheticRequest } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';

const list = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

/** What New Browser Test starts from: a clone of a browser test or the default settings (every 15 minutes at most often). */
export function newBrowserDraft(defaults: DraftDefaults, source?: SyntheticTest): TestDraft {
  if (source) return { ...draftOf(source), name: `${source.name} (clone)`, paused: false };
  const base = newDraft('browser', defaults);
  return { ...base, frequencySeconds: Math.max(base.frequencySeconds, 900) };
}

/** New Browser Test / Edit: starting URL and device, the steps (run once from here), then the steps API tests share. */
export function BrowserTestEditor({ test, start }: { test?: SyntheticTest; start: TestDraft }) {
  const navigate = useNavigate();
  const [d, setD] = useState<TestDraft>(start);
  const [tags, setTags] = useState(() => d.tags.join(', '));
  const [open, setOpen] = useState<number | null>(test ? null : 0);
  const send = useTrySyntheticRequest();
  const save = useSaveSyntheticTest(test?.id);
  const location = useSyntheticLocations().data?.[0];
  const set = (p: Partial<TestDraft>) => setD((x) => ({ ...x, ...p }));
  const setSteps = (steps: BrowserStep[]) => {
    set({ steps });
    send.reset();
  };

  const urlProblem = requestProblem('browser', d.request);
  const detailsOk = !urlProblem && !!d.name.trim();
  const problems = d.steps.map(stepProblem);
  const stepsOk = problems.every((p) => !p);
  const missing = [!detailsOk && 1, !stepsOk && 2].filter(Boolean) as number[];
  const input: SyntheticTestInput = { ...d, tags: list(tags), assertions: [] };
  const browserReady = location?.browser !== false;

  const runOnce = () => send.mutate({ type: 'browser', request: d.request, assertions: [], steps: d.steps });
  const submit = () => save.mutate(input, { onSuccess: (t) => navigate(`/synthetics/tests/${t.id}`) });
  const update = (i: number, s: BrowserStep) => setSteps(d.steps.map((x, j) => (j === i ? s : x)));
  const add = (type: BrowserStepType) => {
    setSteps([...d.steps, newStep(type)]);
    setOpen(d.steps.length);
  };
  const move = (i: number, by: number) => {
    const steps = [...d.steps];
    const [s] = steps.splice(i, 1);
    steps.splice(i + by, 0, s);
    setSteps(steps);
    setOpen(i + by);
  };
  const tried = send.data;
  // Results line up with the steps (result 0 is the starting URL).
  const tryStatus = (i: number) => tried?.result.browser?.steps[i + 1]?.status;

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
          <span>New Browser Test</span>
        )}
      </header>
      <div className="mon-editor-scroll">
        <div className="syn-editor-grid syn-br-editor">
          <div className="mon-steps">
            {!browserReady && (
              <p className="syn-hint warn">
                <AlertTriangle size={14} /> Browser tests run in oc-browser, which this server doesn't use yet (set OC_BROWSER_URL and OC_BROWSER_TOKEN).
              </p>
            )}
            <Step n={1} title="Set Your Test Details" done={detailsOk}>
              <div className="syn-name-grid">
                <label className="form-label syn-span-2">
                  Starting URL*
                  <input
                    className="syn-input mono"
                    value={d.request.url}
                    onChange={(e) => {
                      set({ request: { ...d.request, url: e.target.value } });
                      send.reset();
                    }}
                    placeholder="https://shop.example.com"
                  />
                </label>
                {urlProblem && d.request.url !== 'https://' && <p className="syn-hint warn syn-span-2">{urlProblem}</p>}
                <label className="form-label syn-span-2">
                  Name*
                  <input className="syn-input" value={d.name} onChange={(e) => set({ name: e.target.value })} placeholder="e.g. Checkout flow" />
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
                  <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="service:storefront-api, journey:checkout" />
                </label>
              </div>
              <div className="syn-br-devices" role="radiogroup" aria-label="Device">
                {DEVICES.map((dev) => (
                  <button
                    key={dev.id}
                    role="radio"
                    aria-checked={d.request.deviceId === dev.id}
                    className={`syn-type-btn${d.request.deviceId === dev.id ? ' active' : ''}`}
                    onClick={() => set({ request: { ...d.request, deviceId: dev.id } })}
                  >
                    <MonitorSmartphone size={14} /> {dev.label} <span className="muted">{dev.size}</span>
                  </button>
                ))}
              </div>
              <p className="syn-hint muted">
                Write <code>{'{{ NAME }}'}</code> to use a <Link to="/synthetics/settings/variables">global variable</Link> or a variable an earlier step extracted.
              </p>
            </Step>
            <Step n={2} title="Define Steps" done={stepsOk}>
              <p className="muted syn-text">
                The test opens the starting URL, then runs these steps in a fresh Chromium. Elements are found with their CSS or XPath locators, in order; assertions wait for the page up to their
                timeout (60 s by default).
              </p>
              <ol className="syn-br-edit-steps">
                {d.steps.map((s, i) => (
                  <StepEditor
                    key={i}
                    index={i}
                    step={s}
                    problem={problems[i]}
                    open={open === i}
                    status={tryStatus(i)}
                    count={d.steps.length}
                    onToggle={() => setOpen(open === i ? null : i)}
                    onChange={(x) => update(i, x)}
                    onMove={(by) => move(i, by)}
                    onDuplicate={() => setSteps([...d.steps.slice(0, i + 1), structuredClone(s), ...d.steps.slice(i + 1)])}
                    onRemove={() => {
                      setSteps(d.steps.filter((_, j) => j !== i));
                      setOpen(null);
                    }}
                  />
                ))}
              </ol>
              <div className="syn-row">
                <Menu
                  align="left"
                  triggerClassName="btn"
                  trigger={
                    <>
                      <Plus size={14} /> Add Step
                    </>
                  }
                  items={STEP_GROUPS.flatMap((g) => g.types.map((t) => ({ label: `${g.title}: ${STEP_LABEL[t]}`, onClick: () => add(t) })))}
                />
                <button className="btn btn-primary" disabled={!!urlProblem || !stepsOk || send.isPending || !browserReady} onClick={runOnce}>
                  <Play size={14} /> {send.isPending ? 'Running in the browser…' : 'Run Test'}
                </button>
              </div>
              {send.isError && <p className="form-error">{(send.error as Error).message}</p>}
              {tried && <TryResult result={tried} onClose={() => send.reset()} />}
            </Step>
            <CommonSteps first={3} d={d} set={set} location={location} minFrequency={MIN_BROWSER_FREQUENCY} />
          </div>
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

function TryResult({ result, onClose }: { result: SyntheticTryResult; onClose: () => void }) {
  return (
    <div className="syn-send-result">
      <div className="syn-send-head">
        <span>
          {result.result.passed ? 'Passed' : 'Failed'} in a browser on <b>{result.location.name}</b> (nothing was saved)
        </span>
        <button className="icon-plain" title="Close" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <BrowserRunView
        run={result.result}
        shot={(i) => (result.screenshots[i] ? `data:image/jpeg;base64,${result.screenshots[i]}` : null)}
        video={result.video ? `data:video/webm;base64,${result.video}` : null}
      />
    </div>
  );
}

function StepEditor({
  index,
  step: s,
  problem,
  open,
  status,
  count,
  onToggle,
  onChange,
  onMove,
  onDuplicate,
  onRemove,
}: {
  index: number;
  step: BrowserStep;
  problem: string | null;
  open: boolean;
  status?: string;
  count: number;
  onToggle: () => void;
  onChange: (s: BrowserStep) => void;
  onMove: (by: number) => void;
  onDuplicate: () => void;
  onRemove: () => void;
}) {
  const p = s.params ?? {};
  const setParams = (patch: Partial<NonNullable<BrowserStep['params']>>) => onChange({ ...s, params: { ...p, ...patch } });
  const value = stepValue(s);
  return (
    <li className={`syn-br-edit-step${open ? ' open' : ''}${problem ? ' invalid' : ''}${status ? ` ran-${status}` : ''}`}>
      <div className="syn-br-edit-head">
        <button className="syn-br-edit-title" onClick={onToggle} aria-expanded={open}>
          <span className="syn-br-step-n">{index + 1}</span>
          <span className="syn-chip">{STEP_LABEL[s.type]}</span>
          <span className="syn-br-edit-name">{stepName(s)}</span>
          {problem && !open && <AlertTriangle size={14} className="syn-br-warn" aria-label={problem} />}
        </button>
        <span className="syn-br-edit-actions">
          <button className="icon-plain" title="Move up" disabled={index === 0} onClick={() => onMove(-1)}>
            <ArrowUp size={14} />
          </button>
          <button className="icon-plain" title="Move down" disabled={index === count - 1} onClick={() => onMove(1)}>
            <ArrowDown size={14} />
          </button>
          <button className="icon-plain" title="Duplicate" onClick={onDuplicate}>
            <Copy size={14} />
          </button>
          <button className="icon-plain" title="Delete" onClick={onRemove}>
            <Trash2 size={14} />
          </button>
        </span>
      </div>
      {open && (
        <div className="syn-br-edit-body">
          <label className="form-label">
            Name
            <input className="syn-input" value={s.name ?? ''} onChange={(e) => onChange({ ...s, name: e.target.value })} placeholder={stepName({ ...s, name: '' })} />
          </label>
          {needsElement(s.type) && <ElementEditor element={p.element ?? undefined} onChange={(element) => setParams({ element })} />}
          {s.type === 'goToUrl' && <Field label="URL" value={value} mono onChange={(v) => setParams({ value: v })} placeholder="https://shop.example.com/cart" />}
          {s.type === 'typeText' && (
            <>
              <Field label="Text" value={value} onChange={(v) => setParams({ value: v })} placeholder="{{ USER_EMAIL }}" />
              <label className="mon-check">
                <input type="checkbox" className="checkbox" checked={!!p.appendToContent} onChange={() => setParams({ appendToContent: !p.appendToContent })} />
                <span className="mon-check-label">Append to what the field holds (otherwise it is cleared first)</span>
              </label>
            </>
          )}
          {s.type === 'selectOption' && <Field label="Option (value or label)" value={value} onChange={(v) => setParams({ value: v })} />}
          {s.type === 'click' && (
            <label className="form-label">
              Click
              <select className="syn-select" value={p.clickType ?? 'primary'} onChange={(e) => setParams({ clickType: e.target.value as 'primary' | 'double' | 'contextual' })}>
                <option value="primary">Primary click</option>
                <option value="double">Double click</option>
                <option value="contextual">Right click</option>
              </select>
            </label>
          )}
          {s.type === 'pressKey' && (
            <>
              <label className="form-label">
                Key
                <input className="syn-input" list="syn-br-keys" value={value} onChange={(e) => setParams({ value: e.target.value })} />
                <datalist id="syn-br-keys">
                  {KEYS.map((k) => (
                    <option key={k} value={k} />
                  ))}
                </datalist>
              </label>
              <div className="syn-row">
                {MODIFIERS.map((m) => (
                  <label key={m} className="mon-check">
                    <input
                      type="checkbox"
                      className="checkbox"
                      checked={(p.modifiers ?? []).includes(m)}
                      onChange={() => setParams({ modifiers: (p.modifiers ?? []).includes(m) ? (p.modifiers ?? []).filter((x) => x !== m) : [...(p.modifiers ?? []), m] })}
                    />
                    <span className="mon-check-label">{m}</span>
                  </label>
                ))}
              </div>
            </>
          )}
          {s.type === 'scroll' && (
            <div className="syn-row">
              <Field label="Right (px)" value={String(p.x ?? 0)} onChange={(v) => setParams({ x: Number(v) || 0 })} />
              <Field label="Down (px)" value={String(p.y ?? 0)} onChange={(v) => setParams({ y: Number(v) || 0 })} />
            </div>
          )}
          {s.type === 'wait' && <Field label="Seconds (1–300)" value={value} onChange={(v) => setParams({ value: v })} />}
          {s.type === 'assertElementAttribute' && <Field label="Attribute" value={p.attribute ?? ''} onChange={(v) => setParams({ attribute: v })} placeholder="href" />}
          {usesCheck(s.type) && (
            <div className="syn-row syn-br-check">
              <select className="syn-select" value={p.check ?? 'equals'} onChange={(e) => setParams({ check: e.target.value as (typeof CHECKS)[number] })} aria-label="Check">
                {CHECKS.map((c) => (
                  <option key={c} value={c}>
                    {CHECK_LABEL[c]}
                  </option>
                ))}
              </select>
              {p.check !== 'isEmpty' && p.check !== 'notIsEmpty' && <input className="syn-input" value={value} onChange={(e) => setParams({ value: e.target.value })} aria-label="Expected value" />}
            </div>
          )}
          {(s.type === 'assertPageContains' || s.type === 'assertPageLacks') && <Field label="Text" value={value} onChange={(v) => setParams({ value: v })} placeholder="Order confirmed" />}
          {(s.type === 'assertFromJavascript' || s.type === 'extractFromJavascript') && (
            <label className="form-label">
              JavaScript (a function body; {s.type === 'assertFromJavascript' ? 'a truthy return passes' : 'its return value is the variable'})
              <textarea className="syn-textarea mono" rows={4} value={p.code ?? ''} onChange={(e) => setParams({ code: e.target.value })} />
            </label>
          )}
          {(s.type === 'extractVariable' || s.type === 'extractFromJavascript') && (
            <div className="syn-row">
              <Field
                label="Variable name"
                value={p.variable?.name ?? ''}
                mono
                onChange={(v) => setParams({ variable: { secure: !!p.variable?.secure, name: v.toUpperCase() } })}
                placeholder="ORDER_ID"
              />
              <label className="mon-check">
                <input type="checkbox" className="checkbox" checked={!!p.variable?.secure} onChange={() => setParams({ variable: { name: p.variable?.name ?? '', secure: !p.variable?.secure } })} />
                <span className="mon-check-label">Secure (hidden in results)</span>
              </label>
            </div>
          )}
          <div className="syn-row syn-br-step-options">
            <label className="mon-check">
              <input
                type="checkbox"
                className="checkbox"
                checked={!!s.allowFailure}
                onChange={() => onChange({ ...s, allowFailure: !s.allowFailure, isCritical: s.allowFailure ? false : s.isCritical })}
              />
              <span className="mon-check-label">Continue if this step fails</span>
            </label>
            {s.allowFailure && (
              <label className="mon-check">
                <input type="checkbox" className="checkbox" checked={!!s.isCritical} onChange={() => onChange({ ...s, isCritical: !s.isCritical })} />
                <span className="mon-check-label">…but fail the test</span>
              </label>
            )}
            <label className="mon-check">
              <input type="checkbox" className="checkbox" checked={!!s.noScreenshot} onChange={() => onChange({ ...s, noScreenshot: !s.noScreenshot })} />
              <span className="mon-check-label">No screenshot</span>
            </label>
            <label>
              Timeout
              <input
                className="mon-num"
                value={s.timeout ?? ''}
                placeholder="60"
                inputMode="numeric"
                onChange={(e) => onChange({ ...s, timeout: e.target.value ? Number(e.target.value) || 0 : null })}
                aria-label="Timeout in seconds"
              />
              s
            </label>
          </div>
          {problem && <p className="syn-hint warn">{problem}</p>}
        </div>
      )}
    </li>
  );
}

function Field({ label, value, onChange, placeholder, mono }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; mono?: boolean }) {
  return (
    <label className="form-label">
      {label}
      <input className={`syn-input${mono ? ' mono' : ''}`} value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} />
    </label>
  );
}

/** The element's locators (CSS or XPath, tried in order) and an optional description. */
function ElementEditor({ element, onChange }: { element?: BrowserElement; onChange: (e: BrowserElement) => void }) {
  const values = element?.userLocator?.values ?? [];
  const setValues = (v: typeof values) => onChange({ ...element, userLocator: { failTestOnCannotLocate: true, values: v } });
  return (
    <div className="syn-br-element">
      <span className="form-label">Element (locators tried in order)</span>
      {values.map((l, i) => (
        <div key={i} className="syn-row syn-br-locator">
          <select className="syn-select" value={l.type} onChange={(e) => setValues(values.map((x, j) => (j === i ? { ...x, type: e.target.value as 'css' | 'xpath' } : x)))} aria-label="Locator type">
            <option value="css">CSS</option>
            <option value="xpath">XPath</option>
          </select>
          <input
            className="syn-input mono"
            value={l.value}
            onChange={(e) => setValues(values.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
            placeholder={l.type === 'css' ? 'button[data-test="checkout"]' : "//button[text()='Checkout']"}
            aria-label="Locator"
          />
          {values.length > 1 && (
            <button className="icon-plain" title="Remove the locator" onClick={() => setValues(values.filter((_, j) => j !== i))}>
              <X size={14} />
            </button>
          )}
        </div>
      ))}
      <button className="link-btn" onClick={() => setValues([...values, { type: 'css', value: '' }])} disabled={values.length >= 10}>
        + Add a fallback locator
      </button>
    </div>
  );
}

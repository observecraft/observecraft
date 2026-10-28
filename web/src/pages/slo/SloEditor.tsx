import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Activity, AlertTriangle, Check, ChevronRight, Gauge, Search, Target, Timer } from 'lucide-react';
import type { SloComparator, SloInput, SloItem, SloTarget, SloType, SloWindow } from '../../api/slos/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { formulaQueries } from '../../lib/metricQuery';
import { Method, QueryBuilder, Step } from '../monitors/MonitorEditor';
import { StatusBadge } from '../monitors/common';
import { useMonitors } from '../monitors/queries';
import { WINDOW_LABEL } from './common';
import { useSaveSlo, useSlo } from './queries';
import '../monitors/monitors.css';
import './slo.css';

const TARGET_WINDOWS: SloWindow[] = ['7d', '30d', '90d'];
const COMPARATORS: SloComparator[] = ['<', '<=', '>', '>='];

interface TargetRow {
  on: boolean;
  target: string;
  warning: string;
}

interface Form {
  name: string;
  description: string;
  type: SloType;
  goodQuery: string;
  totalQuery: string;
  monitorIds: number[];
  sliceQuery: string;
  comparator: SloComparator;
  threshold: string;
  sliceMinutes: number;
  targets: Record<SloWindow, TargetRow>;
  primary: SloWindow;
  tags: string;
  teams: string;
}

const num = (s: string) => (s.trim() === '' || Number.isNaN(Number(s)) ? null : Number(s));
const list = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

function initial(slo?: SloItem): Form {
  const targets = Object.fromEntries((['1d', ...TARGET_WINDOWS] as SloWindow[]).map((w) => [w, { on: false, target: '99.9', warning: '' }])) as Record<SloWindow, TargetRow>;
  if (!slo) {
    targets['30d'] = { on: true, target: '99.9', warning: '99.95' };
    return {
      name: '',
      description: '',
      type: 'metric',
      goodQuery: 'sum:trace.http.request.hits{service:order-service,!http.status_code:5*}',
      totalQuery: 'sum:trace.http.request.hits{service:order-service}',
      monitorIds: [],
      sliceQuery: 'avg:trace.latency.p95{service:storefront-api}',
      comparator: '<',
      threshold: '0.5',
      sliceMinutes: 5,
      targets,
      primary: '30d',
      tags: '',
      teams: '',
    };
  }
  for (const t of slo.targets) targets[t.window] = { on: true, target: String(t.target), warning: t.warning === null || t.warning === undefined ? '' : String(t.warning) };
  return {
    name: slo.name,
    description: slo.description,
    type: slo.type,
    goodQuery: slo.goodQuery ?? '',
    totalQuery: slo.totalQuery ?? '',
    monitorIds: slo.monitorIds,
    sliceQuery: slo.sliceQuery ?? '',
    comparator: slo.comparator ?? '<',
    threshold: slo.threshold === null || slo.threshold === undefined ? '' : String(slo.threshold),
    sliceMinutes: slo.sliceMinutes ?? 5,
    targets,
    primary: slo.primaryWindow,
    tags: slo.tags.join(', '),
    teams: slo.teams.join(', '),
  };
}

/** Create (`/slo/create`) or edit (`/slo/:id/edit`) an SLO. */
export function SloEditor() {
  const { id } = useParams();
  const existing = useSlo(id ?? null);
  if (id && existing.isPending) return <Loading label="Loading SLO…" />;
  if (id && existing.isError) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;
  return <EditorForm key={id ?? 'new'} slo={id ? existing.data?.slo : undefined} />;
}

function EditorForm({ slo }: { slo?: SloItem }) {
  const navigate = useNavigate();
  const save = useSaveSlo(slo?.id);
  const [form, setForm] = useState<Form>(() => initial(slo));
  const set = (p: Partial<Form>) => setForm((f) => ({ ...f, ...p }));
  const setTarget = (w: SloWindow, p: Partial<TargetRow>) => setForm((f) => ({ ...f, targets: { ...f.targets, [w]: { ...f.targets[w], ...p } } }));

  const windows = (Object.keys(form.targets) as SloWindow[]).filter((w) => form.targets[w].on);
  const targets: SloTarget[] = windows.map((w) => ({ window: w, target: num(form.targets[w].target) ?? NaN, warning: num(form.targets[w].warning) }));
  const primary = windows.includes(form.primary) ? form.primary : windows[0];
  const sliOk =
    form.type === 'metric'
      ? !!formulaQueries(form.goodQuery) && !!formulaQueries(form.totalQuery)
      : form.type === 'monitor'
        ? form.monitorIds.length > 0
        : !!formulaQueries(form.sliceQuery) && num(form.threshold) !== null;
  const targetsOk = targets.length > 0 && targets.every((t) => t.target > 0 && t.target < 100 && (t.warning === null || t.warning === undefined || (t.warning > t.target && t.warning < 100)));
  const missing = [!sliOk && 1, !targetsOk && 2, !form.name.trim() && 3].filter(Boolean) as number[];

  const input: SloInput = {
    name: form.name,
    description: form.description,
    type: form.type,
    goodQuery: form.type === 'metric' ? form.goodQuery : null,
    totalQuery: form.type === 'metric' ? form.totalQuery : null,
    monitorIds: form.type === 'monitor' ? form.monitorIds : [],
    sliceQuery: form.type === 'time_slice' ? form.sliceQuery : null,
    comparator: form.type === 'time_slice' ? form.comparator : null,
    threshold: form.type === 'time_slice' ? num(form.threshold) : null,
    sliceMinutes: form.type === 'time_slice' ? form.sliceMinutes : null,
    targets,
    primaryWindow: primary ?? null,
    tags: list(form.tags),
    teams: list(form.teams),
  };
  const submit = () => save.mutate(input, { onSuccess: (s) => navigate(`/slo?slo=${encodeURIComponent(s.id)}`) });

  return (
    <div className="mon-editor">
      <header className="mon-crumbs">
        <Link to="/slo" className="mon-crumb-root">
          <Target size={22} /> SLOs
        </Link>
        <ChevronRight size={18} className="muted" />
        {slo ? (
          <>
            <Link to={`/slo?slo=${encodeURIComponent(slo.id)}`} className="mon-crumb-link">
              {slo.name}
            </Link>
            <ChevronRight size={18} className="muted" />
            <span>Edit</span>
          </>
        ) : (
          <span>New SLO</span>
        )}
      </header>
      <div className="mon-editor-scroll">
        <div className="mon-steps">
          <Step n={1} title="Define the SLI" done={sliOk}>
            <div className="mon-methods slo-methods">
              <Method icon={Activity} title="By Count" desc="Good events over total events, from two metric queries." active={form.type === 'metric'} onClick={() => set({ type: 'metric' })} />
              <Method icon={Gauge} title="By Monitor Uptime" desc="Time the selected monitors are not alerting." active={form.type === 'monitor'} onClick={() => set({ type: 'monitor' })} />
              <Method
                icon={Timer}
                title="By Time Slices"
                desc="Share of 1- or 5-minute slices whose value meets a condition."
                active={form.type === 'time_slice'}
                onClick={() => set({ type: 'time_slice' })}
              />
            </div>
            {form.type === 'metric' && (
              <div className="slo-queries">
                <h4>Good events</h4>
                <QueryBuilder type="metric" anyMetric letter="good" query={form.goodQuery} onChange={(goodQuery) => set({ goodQuery })} />
                <h4>Total events</h4>
                <QueryBuilder type="metric" anyMetric letter="total" query={form.totalQuery} onChange={(totalQuery) => set({ totalQuery })} />
                <p className="muted mon-hint">SLI = good ÷ total events over the window. Use counts (requests, hits); formulas between queries work too.</p>
              </div>
            )}
            {form.type === 'monitor' && <MonitorPicker selected={form.monitorIds} onChange={(monitorIds) => set({ monitorIds })} />}
            {form.type === 'time_slice' && (
              <div className="slo-queries">
                <QueryBuilder type="metric" anyMetric query={form.sliceQuery} onChange={(sliceQuery) => set({ sliceQuery })} />
                <div className="mon-eval-row slo-slice-row">
                  <label>
                    A slice is good when the value is
                    <select value={form.comparator} onChange={(e) => set({ comparator: e.target.value as SloComparator })} aria-label="Comparator">
                      {COMPARATORS.map((c) => (
                        <option key={c} value={c}>
                          {c}
                        </option>
                      ))}
                    </select>
                  </label>
                  <input className="mon-num" value={form.threshold} onChange={(e) => set({ threshold: e.target.value })} inputMode="decimal" aria-label="Threshold" />
                  <label>
                    per
                    <select value={form.sliceMinutes} onChange={(e) => set({ sliceMinutes: Number(e.target.value) })} aria-label="Slice">
                      <option value={1}>1-minute</option>
                      <option value={5}>5-minute</option>
                    </select>
                    slice
                  </label>
                </div>
                <p className="muted mon-hint">Slices without data don't count. Long windows use longer slices when they would hold more than 10,080.</p>
              </div>
            )}
          </Step>
          <Step n={2} title="Set your targets" done={targetsOk}>
            <table className="slo-target-table">
              <thead>
                <tr>
                  <th>Time window</th>
                  <th>Target</th>
                  <th>Warning (optional)</th>
                  <th>Primary</th>
                </tr>
              </thead>
              <tbody>
                {TARGET_WINDOWS.map((w) => {
                  const t = form.targets[w];
                  return (
                    <tr key={w} className={t.on ? '' : 'off'}>
                      <td>
                        <label className="mon-check">
                          <input type="checkbox" className="checkbox" checked={t.on} onChange={() => setTarget(w, { on: !t.on })} />
                          <span className="mon-check-label">{WINDOW_LABEL[w]}</span>
                        </label>
                      </td>
                      <td>
                        <span className="slo-pct-input">
                          <input value={t.target} onChange={(e) => setTarget(w, { target: e.target.value })} disabled={!t.on} inputMode="decimal" aria-label={`${w} target`} />%
                        </span>
                      </td>
                      <td>
                        <span className="slo-pct-input">
                          <input value={t.warning} onChange={(e) => setTarget(w, { warning: e.target.value })} disabled={!t.on} inputMode="decimal" placeholder="—" aria-label={`${w} warning`} />%
                        </span>
                      </td>
                      <td>
                        <input type="radio" name="primary" checked={primary === w} disabled={!t.on} onChange={() => set({ primary: w })} aria-label={`${w} primary`} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <p className="muted mon-hint">The warning threshold sits above the target: below it the SLO shows Warning before it breaches. The primary window is what the list shows by default.</p>
          </Step>
          <Step n={3} title="Name and tag the SLO" done={!!form.name.trim()} last>
            <div className="slo-name-form">
              <label className="form-label">
                Name
                <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="e.g. Checkout API availability" />
              </label>
              <label className="form-label">
                Description
                <textarea rows={3} value={form.description} onChange={(e) => set({ description: e.target.value })} placeholder="What the SLO promises and to whom" />
              </label>
              <label className="form-label">
                Tags
                <input value={form.tags} onChange={(e) => set({ tags: e.target.value })} placeholder="service:order-service, env:prod" />
              </label>
              <label className="form-label">
                Teams
                <input value={form.teams} onChange={(e) => set({ teams: e.target.value })} placeholder="checkout, payments" />
              </label>
            </div>
          </Step>
        </div>
      </div>
      <footer className="mon-editor-foot">
        {missing.length ? (
          <span className="mon-foot-warn">
            <AlertTriangle size={20} /> Complete step{missing.length > 1 ? 's' : ''} {missing.join(' and ')}
          </span>
        ) : (
          <span className="mon-foot-ok">
            <Check size={18} /> Ready to {slo ? 'save' : 'create'}
          </span>
        )}
        {save.isError && <span className="form-error">{(save.error as Error).message}</span>}
        <span className="mon-foot-actions">
          <button className="btn" onClick={() => navigate(slo ? `/slo?slo=${encodeURIComponent(slo.id)}` : '/slo')}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={missing.length > 0 || save.isPending} onClick={submit}>
            {slo ? 'Save' : 'Create'}
          </button>
        </span>
      </footer>
    </div>
  );
}

function MonitorPicker({ selected, onChange }: { selected: number[]; onChange: (ids: number[]) => void }) {
  const [filter, setFilter] = useState('');
  const monitors = useMonitors({ query: filter, statuses: [], muted: [], priorities: [], types: [], drafts: [], recipients: [], creators: [], teams: [], sort: 'name', offset: 0, limit: 100 });
  const flip = (id: number) => onChange(selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
  return (
    <div className="slo-monitor-picker">
      <label className="mon-facet-search">
        <Search size={14} />
        <input placeholder="Search monitors" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </label>
      <p className="muted">{selected.length} selected (up to 20). The SLO is down while any of them is in alert.</p>
      <div className="slo-monitor-list">
        {monitors.isPending ? (
          <Loading label="Loading monitors…" />
        ) : (
          (monitors.data?.monitors ?? []).map((m) => (
            <label key={m.id} className="mon-check">
              <input type="checkbox" className="checkbox" checked={selected.includes(m.id)} onChange={() => flip(m.id)} disabled={!selected.includes(m.id) && selected.length >= 20} />
              <StatusBadge status={m.status} small />
              <span className="mon-check-label">{m.name}</span>
            </label>
          ))
        )}
      </div>
    </div>
  );
}

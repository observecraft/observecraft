import { useDeferredValue, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import {
  AlertTriangle,
  BarChart3,
  Check,
  ChevronDown,
  ChevronRight,
  Code2,
  Copy,
  Download,
  EyeOff,
  Gauge,
  Grid3x3,
  LineChart,
  Search,
  Sigma,
  TrendingUp,
  Waves,
  X,
  type LucideIcon,
} from 'lucide-react';
import { metricCatalog } from '../../api/dashboards/client';
import { previewMonitor, sendTestNotification, type MonitorInput, type MonitorOptions, type MonitorType } from '../../api/monitors/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { formatMetricQuery, formulaQueries, parseMetricQuery } from '../../lib/metricQuery';
import { palette } from '../../theme/chartTheme';
import { useTeams } from '../settings/queries';
import { fmtValue, RecipientIcon, recipientLabel } from './common';
import { useMonitor, useSaveMonitor } from './queries';
import { TEMPLATES, type MonitorTemplate } from './templates';
import { messageRecipients, renderMessage } from './message';
import './monitors.css';

const DEFAULT_QUERY: Record<MonitorType, string> = {
  metric: 'avg:system.load.1{*}',
  apm: 'avg:trace.error_rate{env:prod} by {service}',
  log: 'logs("status:error") by {service}',
  // Created with their synthetic test, never in this editor.
  synthetics: '',
};

/** `logs("<search>") by {a,b}` ⇄ parts (mirrors the server's parser). */
export function parseLogQuery(q: string): { search: string; by: string[] } | null {
  const m = /^\s*logs\("((?:[^"\\]|\\.)*)"\)\s*(?:by\s*\{([^}]*)\})?\s*$/.exec(q);
  if (!m) return null;
  return {
    search: m[1].replace(/\\(.)/g, '$1'),
    by: (m[2] ?? '')
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean),
  };
}

export function formatLogQuery(search: string, by: string[]): string {
  const esc = search.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return `logs("${esc}")${by.length ? ` by {${by.join(',')}}` : ''}`;
}
const WINDOWS = [1, 5, 10, 15, 30, 60, 240, 1440];
const CHANGE_PERIODS = [5, 15, 30, 60, 240, 1440, 10_080];
const RANGES = [
  { key: '1h', label: 'Past 1 Hour', ms: 3600e3 },
  { key: '4h', label: 'Past 4 Hours', ms: 4 * 3600e3 },
  { key: '1d', label: 'Past 1 Day', ms: 24 * 3600e3 },
  { key: '1w', label: 'Past 7 Days', ms: 7 * 24 * 3600e3 },
];

function windowLabel(m: number): string {
  return m % 1440 === 0 ? `${m / 1440} day${m === 1440 ? '' : 's'}` : m % 60 === 0 ? `${m / 60} hour${m === 60 ? '' : 's'}` : `${m} minute${m === 1 ? '' : 's'}`;
}

interface Form {
  name: string;
  type: MonitorType;
  query: string;
  options: MonitorOptions;
  /** Thresholds as typed (may be empty or invalid while editing). */
  critical: string;
  warning: string;
  criticalRecovery: string;
  warningRecovery: string;
  message: string;
  priority: number | null;
  tags: string;
  teams: string;
}

function baseOptions(change = false): MonitorOptions {
  return {
    detection: change ? 'change' : 'threshold',
    timeAggregation: 'avg',
    windowMinutes: 5,
    comparator: 'above',
    thresholds: { critical: 0 },
    notifyNoData: false,
    noDataMinutes: 10,
    changeMode: change ? 'pct_change' : null,
    changeMinutes: change ? 60 : null,
  };
}

const num = (s: string) => (s.trim() === '' || Number.isNaN(Number(s)) ? null : Number(s));
const str = (v: number | null | undefined) => (v === null || v === undefined ? '' : String(v));

function fromTemplate(t: MonitorTemplate): Partial<Form> {
  return {
    name: t.name,
    type: t.type,
    query: t.query,
    options: t.options,
    critical: str(t.options.thresholds.critical),
    warning: str(t.options.thresholds.warning),
    criticalRecovery: str(t.options.thresholds.criticalRecovery),
    warningRecovery: str(t.options.thresholds.warningRecovery),
    message: t.message,
  };
}

/** Create (`/monitors/create/:type`) or edit (`/monitors/:id/edit`) a metric or APM monitor. */
export function MonitorEditor() {
  const { type: routeType, id } = useParams();
  const editing = id ? Number(id) : undefined;
  const existing = useMonitor(editing ?? 0, !!editing);
  if (editing && existing.isPending) return <Loading label="Loading monitor…" />;
  if (editing && existing.isError) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;
  const type: MonitorType = routeType === 'apm' || routeType === 'log' ? routeType : 'metric';
  return <EditorForm key={editing ?? routeType} editing={editing} detail={editing ? existing.data : undefined} routeType={type} />;
}

function EditorForm({ editing, detail, routeType }: { editing?: number; detail?: ReturnType<typeof useMonitor>['data']; routeType: MonitorType }) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const save = useSaveMonitor(editing);
  const [form, setForm] = useState<Form>(() => {
    if (detail) {
      const o = detail.options ?? baseOptions();
      return {
        name: detail.name,
        type: detail.type,
        query: detail.metricQuery,
        options: o,
        critical: str(o.thresholds.critical),
        warning: str(o.thresholds.warning),
        criticalRecovery: str(o.thresholds.criticalRecovery),
        warningRecovery: str(o.thresholds.warningRecovery),
        message: detail.message,
        priority: detail.priority ?? null,
        tags: detail.tags.join(', '),
        teams: detail.teams.join(', '),
      };
    }
    const base: Form = {
      name: '',
      type: routeType,
      query: DEFAULT_QUERY[routeType],
      // Log monitors count matching logs over the window.
      options: { ...baseOptions(params.get('detection') === 'change'), timeAggregation: routeType === 'log' ? 'sum' : 'avg' },
      critical: '',
      warning: '',
      criticalRecovery: '',
      warningRecovery: '',
      message: '',
      priority: null,
      tags: '',
      teams: '',
    };
    const tpl = TEMPLATES.find((t) => t.id === params.get('template'));
    if (tpl) return { ...base, ...fromTemplate(tpl) };
    // Prefilled from a link, e.g. an SLO's burn rate alert: ?query=…&name=…&critical=…&window=60&aggregation=avg.
    const prefill = params.get('query');
    if (!prefill) return base;
    const aggregation = params.get('aggregation');
    const window = Number(params.get('window'));
    return {
      ...base,
      query: prefill,
      name: params.get('name') ?? '',
      critical: params.get('critical') ?? '',
      warning: params.get('warning') ?? '',
      message: params.get('message') ?? '',
      tags: params.get('tags') ?? '',
      options: {
        ...base.options,
        windowMinutes: WINDOWS.includes(window) ? window : base.options.windowMinutes,
        timeAggregation: aggregation === 'avg' || aggregation === 'sum' || aggregation === 'max' || aggregation === 'min' ? aggregation : base.options.timeAggregation,
      },
    };
  });
  const [showTemplates, setShowTemplates] = useState(!detail && !params.get('query'));
  const [testing, setTesting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const set = (p: Partial<Form>) => setForm((f) => ({ ...f, ...p }));
  const setOpt = (p: Partial<MonitorOptions>) => setForm((f) => ({ ...f, options: { ...f.options, ...p } }));

  const isLog = form.type === 'log';
  const parsedLog = isLog ? parseLogQuery(form.query) : null;
  const parsed = isLog ? (parsedLog ? { by: parsedLog.by } : null) : (formulaQueries(form.query)?.[0] ?? null);
  const critical = num(form.critical);
  const thresholds = {
    critical: critical ?? 0,
    warning: num(form.warning),
    criticalRecovery: num(form.criticalRecovery),
    warningRecovery: num(form.warningRecovery),
  };
  const options: MonitorOptions = { ...form.options, thresholds };
  const input: MonitorInput = {
    name: form.name.trim(),
    type: form.type,
    query: form.query.trim(),
    options,
    message: form.message,
    priority: form.priority,
    tags: form.tags
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
    teams: form.teams
      .split(',')
      .map((t) => t.trim())
      .filter(Boolean),
    draft: false,
  };
  const orderError = critical === null ? null : thresholdOrderError(options);
  const missing = [(critical === null || orderError) && 3, !input.name && 4].filter(Boolean) as number[];
  const submit = (draft: boolean) => save.mutate({ ...input, draft }, { onSuccess: (m) => navigate(`/monitors/${m.id}`) });
  const byTags = parsed?.by ?? [];

  return (
    <div className="mon-editor">
      <header className="mon-crumbs">
        <Link to="/monitors" className="mon-crumb-root">
          <Gauge size={22} /> Monitors
        </Link>
        <ChevronRight size={18} className="muted" />
        {editing ? (
          <>
            <Link to={`/monitors/${editing}`} className="mon-crumb-link">
              {detail?.name}
            </Link>
            <ChevronRight size={18} className="muted" />
            <span>Edit</span>
          </>
        ) : (
          <>
            <Link to="/monitors/create" className="mon-crumb-link">
              New Monitor
            </Link>
            <ChevronRight size={18} className="muted" />
            <span>{form.type === 'apm' ? 'APM Monitor' : isLog ? 'Logs Monitor' : 'Metric Monitor'}</span>
          </>
        )}
      </header>
      <div className="mon-editor-scroll">
        <Backtest type={form.type} query={form.query} options={orderError ? { ...options, thresholds: { critical: thresholds.critical } } : options} hasThreshold={critical !== null} />
        {showTemplates && <TemplateStrip type={form.type} onPick={(t) => set(fromTemplate(t))} onHide={() => setShowTemplates(false)} />}
        <div className="mon-steps">
          <Step n={1} title="Choose the detection method" done>
            <div className="mon-methods">
              <Method
                icon={LineChart}
                title="Threshold Alert"
                desc="Trigger when a metric crosses a static threshold value."
                active={options.detection === 'threshold'}
                onClick={() =>
                  setOpt({
                    detection: 'threshold',
                    changeMode: null,
                    changeMinutes: null,
                  })
                }
              />
              <Method
                icon={TrendingUp}
                title="Change Alert"
                desc="Trigger when the delta between values exceeds a threshold."
                active={options.detection === 'change'}
                onClick={() =>
                  setOpt({
                    detection: 'change',
                    changeMode: options.changeMode ?? 'pct_change',
                    changeMinutes: options.changeMinutes ?? 60,
                  })
                }
              />
              <Method icon={Waves} title="Anomaly Detection" desc="Trigger when a metric deviates from its expected pattern." disabled />
              <Method icon={Grid3x3} title="Outliers Alert" desc="Trigger when a group member behaves differently from peers." disabled />
              <Method icon={BarChart3} title="Forecast Alert" desc="Trigger when a metric is predicted to cross a threshold." disabled />
            </div>
          </Step>
          <Step n={2} title={isLog ? 'Define the search query' : 'Define the metric'} done={!!parsed}>
            {isLog ? <LogQueryBuilder query={form.query} onChange={(query) => set({ query })} /> : <QueryBuilder type={form.type} query={form.query} onChange={(query) => set({ query })} />}
            <div className="mon-eval">
              <h4>Evaluation Details</h4>
              <div className="mon-eval-row">
                {options.detection === 'change' && (
                  <label>
                    Compare the
                    <select
                      value={options.changeMode ?? 'pct_change'}
                      onChange={(e) =>
                        setOpt({
                          changeMode: e.target.value as MonitorOptions['changeMode'],
                        })
                      }
                    >
                      <option value="change">change</option>
                      <option value="pct_change">% change</option>
                    </select>
                  </label>
                )}
                <label>
                  {options.detection === 'change' ? 'of the' : 'Evaluate the'}
                  <select
                    value={options.timeAggregation}
                    onChange={(e) =>
                      setOpt({
                        timeAggregation: e.target.value as MonitorOptions['timeAggregation'],
                      })
                    }
                  >
                    <option value="avg">average</option>
                    <option value="max">maximum</option>
                    <option value="min">minimum</option>
                    <option value="sum">sum</option>
                    <option value="last">last value</option>
                  </select>
                </label>
                <label>
                  of the query over the
                  <select value={options.windowMinutes} onChange={(e) => setOpt({ windowMinutes: Number(e.target.value) })}>
                    {WINDOWS.map((w) => (
                      <option key={w} value={w}>
                        last {windowLabel(w)}
                      </option>
                    ))}
                  </select>
                </label>
                {options.detection === 'change' && (
                  <label>
                    compared to
                    <select value={options.changeMinutes ?? 60} onChange={(e) => setOpt({ changeMinutes: Number(e.target.value) })}>
                      {CHANGE_PERIODS.map((w) => (
                        <option key={w} value={w}>
                          {windowLabel(w)} before
                        </option>
                      ))}
                    </select>
                  </label>
                )}
              </div>
              <p className="muted mon-hint">
                Evaluated every minute
                {byTags.length ? `, separately for each ${byTags.join(' × ')}` : ''}.
              </p>
            </div>
          </Step>
          <Step n={3} title="Set alert conditions" done={critical !== null && !orderError}>
            <div className="mon-cond">
              <label className="mon-cond-line">
                Trigger when the evaluated value is
                <select
                  value={options.comparator}
                  onChange={(e) =>
                    setOpt({
                      comparator: e.target.value as MonitorOptions['comparator'],
                    })
                  }
                >
                  <option value="above">above</option>
                  <option value="above_or_equal">above or equal to</option>
                  <option value="below">below</option>
                  <option value="below_or_equal">below or equal to</option>
                </select>
                the threshold
              </label>
              <div className="mon-thresholds">
                <Threshold
                  label="Alert threshold"
                  tone="alert"
                  value={form.critical}
                  onChange={(critical) => set({ critical })}
                  required
                  unit={options.detection === 'change' && options.changeMode === 'pct_change' ? '%' : undefined}
                />
                <Threshold
                  label="Warning threshold"
                  tone="warn"
                  value={form.warning}
                  onChange={(warning) => set({ warning })}
                  unit={options.detection === 'change' && options.changeMode === 'pct_change' ? '%' : undefined}
                />
                <Threshold label="Alert recovery threshold" tone="ok" value={form.criticalRecovery} onChange={(criticalRecovery) => set({ criticalRecovery })} />
                <Threshold label="Warning recovery threshold" tone="ok" value={form.warningRecovery} onChange={(warningRecovery) => set({ warningRecovery })} />
              </div>
              {orderError && <p className="form-error">{orderError}</p>}
              <div className="mon-nodata">
                <select value={options.notifyNoData ? 'notify' : 'quiet'} onChange={(e) => setOpt({ notifyNoData: e.target.value === 'notify' })}>
                  <option value="quiet">Do not notify</option>
                  <option value="notify">Notify</option>
                </select>
                if data is missing for
                <input
                  className="mon-num"
                  type="number"
                  min={1}
                  max={1440}
                  value={options.noDataMinutes ?? 10}
                  disabled={!options.notifyNoData}
                  onChange={(e) => setOpt({ noDataMinutes: Number(e.target.value) })}
                />
                minutes.
              </div>
            </div>
          </Step>
          <Step n={4} title="Configure notifications & automations" done={!!input.name} last>
            <NotificationsStep form={form} set={set} byTags={byTags} />
          </Step>
        </div>
      </div>
      <footer className="mon-editor-foot">
        {missing.length ? (
          <span className="mon-foot-warn">
            <AlertTriangle size={20} /> Complete step
            {missing.length > 1 ? 's' : ''} {missing.join(' and ')}
          </span>
        ) : (
          <span className="mon-foot-ok">
            <Check size={18} /> Ready to {editing ? 'save' : 'create'}
          </span>
        )}
        {save.isError && <span className="form-error">{(save.error as Error).message}</span>}
        <span className="mon-foot-actions">
          <button className="link-muted" disabled={!form.message.trim()} onClick={() => setTesting(true)}>
            Test Notifications
          </button>
          <button className="link-muted" onClick={() => setExporting(true)}>
            Export Monitor
          </button>
          <span className="mon-sep" />
          <button className="btn" disabled={missing.length > 0 || save.isPending} onClick={() => submit(true)}>
            Save as Draft
          </button>
          <button className="btn btn-primary" disabled={missing.length > 0 || save.isPending} onClick={() => submit(false)}>
            {editing ? (detail?.draft ? 'Publish' : 'Save Changes') : 'Create and Publish'}
          </button>
        </span>
      </footer>
      {testing && <TestNotifications input={input} monitorId={editing} byTags={byTags} onClose={() => setTesting(false)} />}
      {exporting && <ExportModal input={input} onClose={() => setExporting(false)} />}
    </div>
  );
}

/** Mirrors the server's check: thresholds ordered in the alerting direction. */
function thresholdOrderError(o: MonitorOptions): string | null {
  const up = o.comparator.startsWith('above');
  const side = up ? 'below' : 'above';
  const before = (a: number, b: number) => (up ? a < b : a > b);
  const t = o.thresholds;
  if (t.warning != null && !before(t.warning, t.critical)) return `The warning threshold must be ${side} the alert threshold.`;
  if (t.criticalRecovery != null && !before(t.criticalRecovery, t.critical)) return `The alert recovery threshold must be ${side} the alert threshold.`;
  if (t.warningRecovery != null && t.warning == null) return 'A warning recovery threshold needs a warning threshold.';
  if (t.warningRecovery != null && t.warning != null && !before(t.warningRecovery, t.warning)) return `The warning recovery threshold must be ${side} the warning threshold.`;
  return null;
}

export function Step({ n, title, done, last, children }: { n: number; title: string; done: boolean; last?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className={`mon-step${last ? ' last' : ''}`}>
      <div className="mon-step-head">
        <span className={`mon-step-n${done ? ' done' : ''}`}>{n}</span>
        <button className="mon-step-title" onClick={() => setOpen((o) => !o)}>
          {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />} {title}
        </button>
      </div>
      {open && <div className="mon-step-body">{children}</div>}
    </section>
  );
}

export function Method({ icon: Icon, title, desc, active, disabled, onClick }: { icon: LucideIcon; title: string; desc: string; active?: boolean; disabled?: boolean; onClick?: () => void }) {
  return (
    <button className={`mon-method${active ? ' active' : ''}`} disabled={disabled} onClick={onClick} title={disabled ? 'Not available yet' : undefined}>
      <Icon size={22} strokeWidth={1.5} />
      <span>
        <b>{title}</b>
        <span className="muted">{desc}</span>
      </span>
    </button>
  );
}

function Threshold({
  label,
  tone,
  value,
  onChange,
  required,
  unit,
}: {
  label: string;
  tone: 'alert' | 'warn' | 'ok';
  value: string;
  onChange: (v: string) => void;
  required?: boolean;
  unit?: string;
}) {
  const invalid = value.trim() !== '' && Number.isNaN(Number(value));
  return (
    <label className={`mon-threshold t-${tone}`}>
      <span>
        {label}
        {required && <i>*</i>}
      </span>
      <span className="mon-threshold-input">
        <input value={value} onChange={(e) => onChange(e.target.value)} inputMode="decimal" aria-invalid={invalid} placeholder={required ? 'required' : 'optional'} />
        {unit && <em>{unit}</em>}
      </span>
    </label>
  );
}

// ================================================================ query builder

/** Logs monitor: a Log Explorer search, counted per group. */
function LogQueryBuilder({ query, onChange }: { query: string; onChange: (q: string) => void }) {
  const parsed = parseLogQuery(query) ?? { search: '', by: [] };
  return (
    <div className="mon-qb">
      <div className="mon-qb-row">
        <span className="mon-qb-letter">a</span>
        <input
          className="mon-qb-raw"
          value={parsed.search}
          onChange={(e) => onChange(formatLogQuery(e.target.value, parsed.by))}
          placeholder='service:checkout status:error "payment declined"'
          spellCheck={false}
          aria-label="Log search"
        />
      </div>
      <div className="mon-qb-row">
        <span className="mon-qb-label">count by</span>
        <input
          className="mon-qb-raw"
          value={parsed.by.join(', ')}
          onChange={(e) =>
            onChange(
              formatLogQuery(
                parsed.search,
                e.target.value
                  .split(',')
                  .map((s) => s.trim())
                  .filter(Boolean),
              ),
            )
          }
          placeholder="(everything) e.g. service, host, @http.status_code"
          aria-label="Group by"
        />
      </div>
      <p className="muted mon-hint">
        Counts the logs matching the search (same syntax as the Log Explorer) per minute; the evaluation window adds them up. Source: <code>{query}</code>
      </p>
    </div>
  );
}

/** Metric query builder; `anyMetric` offers every metric (trace.* included) whatever the type, `letter` labels the row. */
export function QueryBuilder({ type, query, onChange, anyMetric = false, letter = 'a' }: { type: MonitorType; query: string; onChange: (q: string) => void; anyMetric?: boolean; letter?: string }) {
  const [raw, setRaw] = useState(false);
  const catalog = useQuery({
    queryKey: ['metric-catalog'],
    queryFn: ({ signal }) => metricCatalog(signal),
    staleTime: 300_000,
  });
  const parsed = parseMetricQuery(query);
  const metrics = (catalog.data ?? []).filter((m) => anyMetric || (type === 'apm' ? m.name.startsWith('trace.') : !m.name.startsWith('trace.')));
  const entry = catalog.data?.find((m) => m.name === parsed?.metric);
  const update = (p: Partial<NonNullable<typeof parsed>>) => parsed && onChange(formatMetricQuery({ ...parsed, ...p }));
  const from = parsed ? parsed.filters.map((f) => `${f.negate ? '!' : ''}${f.key}:${f.value}`).join(', ') : '';
  return (
    <div className="mon-qb">
      <div className="mon-qb-tabs">
        <button className={!raw ? 'active' : ''} onClick={() => setRaw(false)}>
          Edit
        </button>
        <button className={raw ? 'active' : ''} onClick={() => setRaw(true)}>
          Source
        </button>
      </div>
      {raw || !parsed ? (
        <div className="mon-qb-row">
          <span className="mon-qb-letter">{letter}</span>
          <input className="mon-qb-raw" value={query} onChange={(e) => onChange(e.target.value)} spellCheck={false} aria-label="Metric query" />
          {!formulaQueries(query) && (
            <span className="form-error">
              Expected avg:metric.name{'{tag:value}'} by {'{tag}'}
            </span>
          )}
        </div>
      ) : (
        <div className="mon-qb-row">
          <span className="mon-qb-letter">{letter}</span>
          <input
            className="mon-qb-metric"
            list={`mon-metrics-${letter}`}
            value={parsed.metric}
            onChange={(e) => update({ metric: e.target.value.trim() })}
            aria-label="Metric"
            title={entry?.description}
          />
          <datalist id={`mon-metrics-${letter}`}>
            {metrics.map((m) => (
              <option key={m.name} value={m.name}>
                {m.description}
              </option>
            ))}
          </datalist>
          <span className="mon-qb-kw">from</span>
          <FromInput
            value={from}
            tags={entry?.tags ?? []}
            onChange={(text) => {
              const filters = text
                .split(',')
                .map((s) => s.trim())
                .filter((s) => s && s !== '*' && s.includes(':'))
                .map((s) => {
                  const negate = s.startsWith('!');
                  const body = negate ? s.slice(1) : s;
                  const i = body.indexOf(':');
                  return {
                    key: body.slice(0, i),
                    value: body.slice(i + 1),
                    negate,
                  };
                });
              update({ filters });
            }}
          />
          <select className="mon-qb-agg" value={parsed.aggregator} onChange={(e) => update({ aggregator: e.target.value as typeof parsed.aggregator })} aria-label="Space aggregation">
            <option value="avg">avg by</option>
            <option value="max">max by</option>
            <option value="min">min by</option>
            <option value="sum">sum by</option>
          </select>
          <select className="mon-qb-by" value={parsed.by[0] ?? ''} onChange={(e) => update({ by: e.target.value ? [e.target.value] : [] })} aria-label="Group by">
            <option value="">(everything)</option>
            {(entry?.tags ?? ['host', 'service', 'env']).map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
          <span className="mon-qb-modify" title="Space aggregation across series">
            <Sigma size={15} /> {parsed.aggregator}
          </span>
        </div>
      )}
      <button className="icon-plain mon-qb-code" title={raw ? 'Builder' : 'Edit as text'} onClick={() => setRaw((r) => !r)}>
        <Code2 size={16} />
      </button>
    </div>
  );
}

/** Free-text tag filter that only commits on blur/Enter (typing "env:" shouldn't drop the filter). */
function FromInput({ value, tags, onChange }: { value: string; tags: string[]; onChange: (v: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <input
      className="mon-qb-from"
      placeholder="(everywhere)"
      value={text}
      title={tags.length ? `Tags: ${tags.join(', ')}` : undefined}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onChange(text)}
      onKeyDown={(e) => e.key === 'Enter' && onChange(text)}
      aria-label="Filter tags"
    />
  );
}

// ================================================================ backtest

function Backtest({ type, query, options, hasThreshold }: { type: MonitorType; query: string; options: MonitorOptions; hasThreshold: boolean }) {
  const [mode, setMode] = useState<'evaluated' | 'raw'>('evaluated');
  const [rangeKey, setRangeKey] = useState('1h');
  const valid = type === 'log' ? !!parseLogQuery(query) : !!formulaQueries(query);
  // Without a threshold yet, preview with one that never triggers.
  const effective: MonitorOptions = hasThreshold
    ? options
    : {
        ...options,
        thresholds: {
          critical: options.comparator.startsWith('above') ? 1e300 : -1e300,
        },
      };
  const key = useDeferredValue(JSON.stringify({ type, query, effective, rangeKey }));
  const q = useQuery({
    queryKey: ['monitor-preview', key],
    queryFn: ({ signal }) => {
      const k = JSON.parse(key);
      const to = Date.now();
      return previewMonitor(k.type, k.query, k.effective, { from: to - RANGES.find((r) => r.key === k.rangeKey)!.ms, to }, signal);
    },
    enabled: valid,
    retry: false,
    staleTime: 30_000,
  });
  const t = options.thresholds;
  const option = useMemo<EChartsOption | null>(() => {
    if (!q.data) return null;
    const series = mode === 'raw' ? q.data.raw : q.data.evaluated;
    const colors = palette();
    const lines =
      mode === 'evaluated' && hasThreshold
        ? [
            {
              yAxis: t.critical,
              lineStyle: { color: '#e5383b', type: 'dashed' as const },
              label: { formatter: 'Alert', color: '#e5383b' },
            },
            ...(t.warning !== null && t.warning !== undefined
              ? [
                  {
                    yAxis: t.warning,
                    lineStyle: { color: '#f2a93b', type: 'dashed' as const },
                    label: { formatter: 'Warn', color: '#f2a93b' },
                  },
                ]
              : []),
          ]
        : [];
    return {
      grid: { left: 48, right: 56, top: 16, bottom: 28 },
      tooltip: {
        trigger: 'axis',
        valueFormatter: (v) => fmtValue(v as number),
      },
      xAxis: { type: 'time' },
      // Keep the threshold lines in view even when the data stays far from them.
      yAxis: {
        type: 'value',
        scale: true,
        // The axis can start at a threshold or a data minimum (61.3333…): label it rounded.
        axisLabel: { formatter: (v: number) => fmtValue(v) },
        ...(lines.length
          ? {
              min: (v: { min: number }) => Math.min(v.min, ...lines.map((l) => l.yAxis)),
              max: (v: { max: number }) => Math.max(v.max, ...lines.map((l) => l.yAxis)),
            }
          : {}),
      },
      series: series.map((s, i) => ({
        name: s.group,
        type: 'line',
        showSymbol: false,
        connectNulls: false,
        lineStyle: { width: 1.5 },
        color: colors[i % colors.length],
        data: s.points.map((p) => [p[0], p[1]]),
        ...(i === 0 && lines.length ? { markLine: { symbol: 'none', silent: true, data: lines } } : {}),
      })),
    };
  }, [q.data, mode, hasThreshold, t.critical, t.warning]);
  const tr = q.data?.transitions;
  return (
    <section className="mon-backtest">
      <div className="mon-bt-main">
        <div className="mon-bt-bar">
          <div className="mon-seg small">
            <button className={mode === 'evaluated' ? 'active' : ''} onClick={() => setMode('evaluated')}>
              Backtested Data
            </button>
            <button className={mode === 'raw' ? 'active' : ''} onClick={() => setMode('raw')}>
              Raw Data
            </button>
          </div>
          <select value={rangeKey} onChange={(e) => setRangeKey(e.target.value)} aria-label="Preview range">
            {RANGES.map((r) => (
              <option key={r.key} value={r.key}>
                {r.label}
              </option>
            ))}
          </select>
          {q.data && (
            <span className="muted">
              {q.data.groups === 1 ? '1 group' : `${q.data.groups} groups`}
              {q.data.groups > 20 ? ' (showing 20)' : ''}
            </span>
          )}
        </div>
        {!valid ? (
          <p className="mon-bt-empty">Define a valid query to preview it.</p>
        ) : q.isPending ? (
          <Loading label="Backtesting…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : q.data.groups === 0 ? (
          <p className="mon-bt-empty">No data for this query in the selected range.</p>
        ) : (
          option && <Chart option={option} height={220} />
        )}
      </div>
      <aside className="mon-bt-side">
        <h4>Monitor behavior with this threshold</h4>
        <p className="muted">
          {hasThreshold
            ? `How many times the monitor would have transitioned during the selected timeframe.`
            : 'Add thresholds to see how many times the monitor would have transitioned during the selected timeframe with the current configuration.'}
        </p>
        <ul className="mon-bt-counts">
          <li className="c-alert">{tr?.alert ?? 0} Alert</li>
          <li className="c-warn">{tr?.warn ?? 0} Warn</li>
          <li className="c-ok">{tr?.recover ?? 0} Recover</li>
          <li className="c-nodata">{tr?.nodata ?? 0} No Data</li>
        </ul>
      </aside>
    </section>
  );
}

// ================================================================ templates

function TemplateStrip({ type, onPick, onHide }: { type: MonitorType; onPick: (t: MonitorTemplate) => void; onHide: () => void }) {
  const [q, setQ] = useState('');
  const f = q.trim().toLowerCase();
  const shown = TEMPLATES.filter((t) => (f ? `${t.source} ${t.title} ${t.query}`.toLowerCase().includes(f) : t.type === type)).slice(0, 3);
  return (
    <section className="mon-tpl-strip">
      <div>
        <h3>Start from a monitor template</h3>
        <p className="muted">Templates include search queries, alert conditions, and notifications</p>
        <label className="mon-tpl-search">
          <Search size={16} />
          <input placeholder="Search templates by service, integration, or use case" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
      </div>
      <div className="mon-tpl-cards">
        {shown.map((t) => (
          <button key={t.id} className="mon-tpl-card small" onClick={() => onPick(t)}>
            <span className="mon-tpl-source">{t.source}</span>
            <b>{t.title}</b>
          </button>
        ))}
        {shown.length === 0 && <p className="muted">No template matches.</p>}
      </div>
      <button className="icon-plain mon-tpl-hide" title="Hide templates" onClick={onHide}>
        <EyeOff size={18} />
      </button>
    </section>
  );
}

// ================================================================ notifications

function NotificationsStep({ form, set, byTags }: { form: Form; set: (p: Partial<Form>) => void; byTags: string[] }) {
  const teams = useTeams({ query: '', sort: 'name', offset: 0, limit: 200 });
  const ref = useRef<HTMLTextAreaElement>(null);
  const insert = (text: string) => {
    const el = ref.current;
    const at = el?.selectionStart ?? form.message.length;
    set({
      message: form.message.slice(0, at) + text + form.message.slice(el?.selectionEnd ?? at),
    });
    requestAnimationFrame(() => el?.focus());
  };
  const vars = [
    '{{#is_alert}}{{/is_alert}}',
    '{{#is_warning}}{{/is_warning}}',
    '{{#is_recovery}}{{/is_recovery}}',
    '{{#is_no_data}}{{/is_no_data}}',
    '{{value}}',
    '{{threshold}}',
    ...byTags.map((t) => `{{${t}.name}}`),
  ];
  const recipients = messageRecipients(form.message);
  return (
    <div className="mon-notify">
      <label className="form-label">
        Monitor name<i className="req">*</i>
        <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder={byTags.length ? `e.g. High load on {{${byTags[0]}.name}}` : 'e.g. High load average'} />
      </label>
      <label className="form-label">
        Message
        <textarea
          ref={ref}
          rows={6}
          value={form.message}
          onChange={(e) => set({ message: e.target.value })}
          placeholder={'{{#is_alert}}Load is {{value}} — investigate. @slack-platform-alerts @oncall@example.com{{/is_alert}}'}
          spellCheck={false}
        />
      </label>
      <div className="mon-vars">
        <span className="muted">Insert:</span>
        {vars.map((v) => (
          <button key={v} type="button" className="mon-var" onClick={() => insert(v)} title={`Insert ${v}`}>
            {`${v.split('}}')[0]}}}`}
          </button>
        ))}
      </div>
      <div className="mon-recipients">
        <span className="muted">Recipients from message:</span>
        {recipients.length === 0 ? (
          <span className="muted">none — @-mention a person, team, email, Slack channel or webhook</span>
        ) : (
          recipients.map((r) => (
            <span key={r} className="mon-rcpt-chip">
              <RecipientIcon r={r} /> {recipientLabel(r)}
            </span>
          ))
        )}
      </div>
      <div className="mon-notify-grid">
        <label className="form-label">
          Priority
          <select value={form.priority ?? ''} onChange={(e) => set({ priority: e.target.value ? Number(e.target.value) : null })}>
            <option value="">Not Defined</option>
            {[1, 2, 3, 4, 5].map((p) => (
              <option key={p} value={p}>
                P{p} ({['Critical', 'High', 'Medium', 'Low', 'Info'][p - 1]})
              </option>
            ))}
          </select>
        </label>
        <label className="form-label">
          Tags
          <input value={form.tags} onChange={(e) => set({ tags: e.target.value })} placeholder="team:platform, env:prod, service:billing" />
        </label>
        <label className="form-label">
          Teams
          <input list="mon-teams" value={form.teams} onChange={(e) => set({ teams: e.target.value })} placeholder="platform, sre" />
          <datalist id="mon-teams">
            {(teams.data?.teams ?? []).map((t) => (
              <option key={t.handle} value={t.handle}>
                {t.name}
              </option>
            ))}
          </datalist>
        </label>
      </div>
      <p className="muted mon-hint">Notifications are recorded on the monitor's event timeline. Delivering them to email, chat or on-call needs the matching integration.</p>
    </div>
  );
}

function TestNotifications({ input, monitorId, byTags, onClose }: { input: MonitorInput; monitorId?: number; byTags: string[]; onClose: () => void }) {
  const sample = Object.fromEntries(byTags.map((t) => [t, `<${t}>`]));
  const [results, setResults] = useState<Record<string, { recipient: string; status: string; error?: string | null }[] | string>>({});
  const send = (label: string, status: string, text: string, value: number | null) => {
    setResults((r) => ({ ...r, [label]: 'Sending…' }));
    sendTestNotification({ monitorId, monitorName: input.name || 'Untitled monitor', message: text, status, value })
      .then((out) => setResults((r) => ({ ...r, [label]: out })))
      .catch((e: Error) => setResults((r) => ({ ...r, [label]: e.message })));
  };
  const cases = [
    {
      status: 'alert',
      previous: 'ok',
      label: 'Alert',
      value: input.options.thresholds.critical,
    },
    {
      status: 'warn',
      previous: 'ok',
      label: 'Warn',
      value: input.options.thresholds.warning ?? null,
    },
    { status: 'ok', previous: 'alert', label: 'Alert Recovery', value: null },
    { status: 'nodata', previous: 'ok', label: 'No Data', value: null },
  ] as const;
  return (
    <Modal onClose={onClose} label="Test notifications" width={720} height="auto">
      <div className="small-modal">
        <div className="modal-title-row">
          <h3>Test Notifications</h3>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">What each transition sends, rendered from your message. Send delivers it now to its recipients, marked as a test.</p>
        {cases
          .filter((c) => c.status !== 'warn' || c.value !== null)
          .map((c) => {
            const text = renderMessage(input.message ?? '', {
              status: c.status,
              previous: c.previous,
              value: c.value,
              thresholds: input.options.thresholds,
              monitorName: input.name,
              group: sample,
            });
            const who = messageRecipients(text);
            return (
              <div key={c.label} className="mon-test">
                <b className={`mon-test-label t-${c.status}`}>{c.label}</b>
                <pre>{text.trim() || '(empty message)'}</pre>
                <span className="muted">→ {who.length ? who.map((w) => `@${w}`).join(', ') : 'no recipients'}</span>
                {who.length > 0 && (
                  <button className="btn btn-sm mon-test-send" onClick={() => send(c.label, c.status, text, c.value)} disabled={results[c.label] === 'Sending…'}>
                    Send test
                  </button>
                )}
                {typeof results[c.label] === 'string' && <span className="muted">{results[c.label] as string}</span>}
                {Array.isArray(results[c.label]) && (
                  <ul className="mon-test-results">
                    {(results[c.label] as { recipient: string; status: string; error?: string | null }[]).map((r) => (
                      <li key={r.recipient} className={r.status === 'sent' ? 'ok' : 'bad'}>
                        @{r.recipient}: {r.status === 'sent' ? 'sent' : r.error}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            );
          })}
      </div>
    </Modal>
  );
}

function ExportModal({ input, onClose }: { input: MonitorInput; onClose: () => void }) {
  const json = JSON.stringify(input, null, 2);
  const [copied, setCopied] = useState(false);
  return (
    <Modal onClose={onClose} label="Export monitor" width={720} height="auto">
      <div className="small-modal">
        <div className="modal-title-row">
          <h3>Export Monitor</h3>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <pre className="mon-json">{json}</pre>
        <div className="small-modal-actions">
          <button
            className="btn"
            onClick={() => {
              const a = document.createElement('a');
              a.href = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
              a.download = `${input.name.replace(/[^\w.-]+/g, '-').slice(0, 60) || 'monitor'}.json`;
              a.click();
              URL.revokeObjectURL(a.href);
            }}
          >
            <Download size={14} /> Download
          </button>
          <button
            className="btn btn-primary"
            onClick={() =>
              void navigator.clipboard.writeText(json).then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              })
            }
          >
            <Copy size={14} /> {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

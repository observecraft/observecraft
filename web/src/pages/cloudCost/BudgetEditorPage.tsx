import { useEffect, useMemo, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Check, ChevronDown, ChevronRight, CircleDollarSign, Download, Pencil, Settings, Upload, X } from 'lucide-react';
import { getBudget, saveBudget, type BudgetDoc, type BudgetEntry } from '../../api/cloudCost/client';
import { budgetScope } from '../../api/cloudCost/budgetQuery';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { token } from '../../theme/chartTheme';
import { CLOUDS, DAY, filterQuery, money, moneyAxis, monthStart, providerLabel, tagValues, today, useCosts, useCostTags } from './costModel';
import './cloudCost.css';

type Kind = 'basic' | 'hierarchical';

interface Row {
  /** One value per `by` tag. */
  values: string[];
  amounts: Record<number, number>;
}

interface Draft {
  kind: Kind;
  name: string;
  /** The name follows the settings until it's edited. */
  customName: boolean;
  start: number;
  end: number;
  /** '' = all providers. */
  provider: string;
  by: string[];
  filters: [string, string][];
  rows: Row[];
}

const SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const yyyymm = (ms: number) => new Date(ms).getUTCFullYear() * 100 + new Date(ms).getUTCMonth() + 1;
const msOf = (m: number) => Date.UTC(Math.floor(m / 100), (m % 100) - 1, 1);
const nextMonth = (m: number) => (m % 100 === 12 ? m + 89 : m + 1);
const monthsOf = (a: number, b: number) => {
  const out: number[] = [];
  for (let m = a; m <= b && out.length < 60; m = nextMonth(m)) out.push(m);
  return out;
};
const monthHead = (m: number) => `${SHORT[(m % 100) - 1]} ${Math.floor(m / 100)}`.toUpperCase();
const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** "Jan - Dec 2026 All Providers Cost - Subaccountname Budget". */
function autoName(d: Draft): string {
  const [sy, ey] = [Math.floor(d.start / 100), Math.floor(d.end / 100)];
  const period = sy === ey ? `${SHORT[(d.start % 100) - 1]} - ${SHORT[(d.end % 100) - 1]} ${sy}` : `${SHORT[(d.start % 100) - 1]} ${sy} - ${SHORT[(d.end % 100) - 1]} ${ey}`;
  const provider = d.provider ? providerLabel(d.provider) : 'All Providers';
  return `${period} ${provider} Cost - ${d.by.map(capital).join(' / ') || 'Total'} Budget`;
}

const quoteValue = (v: string) => (/[\s,{}:]/.test(v) ? `"${v}"` : v);

/** The budget's `metrics_query`: the provider's amortized cost, the filters, `by` its tags. */
function metricsQuery(d: Draft): string {
  const filters = d.filters.map(([k, v]) => `${k}:${quoteValue(v)}`);
  let metric = 'all.cost';
  if (CLOUDS.includes(d.provider)) metric = `${d.provider}.cost.amortized`;
  else if (d.provider) filters.unshift(`providername:${quoteValue(d.provider)}`);
  return `${metric}{${filters.join(',') || '*'}} by {${d.by.join(',')}}`;
}

/** A saved budget as a draft: its provider, filters and tags back from the query. */
function draftOf(doc: BudgetDoc): Draft {
  const a = doc.attributes;
  const m = /^\s*(?:\w+:)?([\w.]+)\s*\{([^}]*)\}\s*(?:by\s*\{([^}]*)\})?/.exec(a.metrics_query);
  const metric = m?.[1] ?? 'all.cost';
  let provider = metric === 'all.cost' ? '' : metric.split('.cost.')[0];
  const filters: [string, string][] = [];
  for (const f of (m?.[2] ?? '').split(',').map((x) => x.trim())) {
    const i = f.indexOf(':');
    if (!f || f === '*' || i < 0) continue;
    const [k, v] = [f.slice(0, i), f.slice(i + 1).replace(/^"|"$/g, '')];
    if (k === 'providername' && !provider) provider = v;
    else filters.push([k, v]);
  }
  const by = (m?.[3] ?? '')
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);
  const rows = new Map<string, Row>();
  for (const e of a.entries ?? []) {
    const values = by.map((k) => e.tag_filters.find((f) => f.tag_key === k)?.tag_value ?? '');
    const key = JSON.stringify(values);
    const row = rows.get(key) ?? { values, amounts: {} };
    row.amounts[e.month] = e.amount;
    rows.set(key, row);
  }
  return { kind: by.length > 1 ? 'hierarchical' : 'basic', name: a.name, customName: true, start: a.start_month, end: a.end_month, provider, by, filters, rows: [...rows.values()] };
}

function newDraft(): Draft {
  const y = new Date(today()).getUTCFullYear();
  const d: Draft = { kind: 'basic', name: '', customName: false, start: y * 100 + 1, end: y * 100 + 12, provider: '', by: ['subaccountname'], filters: [], rows: [] };
  return { ...d, name: autoName(d) };
}

const rowKey = (values: string[]) => JSON.stringify(values);

function csvCell(v: string | number): string {
  const s = String(v);
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function parseCsv(text: string): string[][] {
  const out: string[][] = [];
  let row: string[] = [];
  let cell = '';
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') ((cell += '"'), i++);
      else if (c === '"') quoted = false;
      else cell += c;
    } else if (c === '"') quoted = true;
    else if (c === ',') (row.push(cell), (cell = ''));
    else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(cell);
      if (row.some((x) => x.trim())) out.push(row);
      ((row = []), (cell = ''));
    } else cell += c;
  }
  row.push(cell);
  if (row.some((x) => x.trim())) out.push(row);
  return out;
}

function MonthSelect({ value, onChange, months }: { value: number; onChange: (m: number) => void; months: number[] }) {
  return (
    <Select
      value={String(value)}
      options={months.map((m) => ({ value: String(m), label: `${LONG[(m % 100) - 1]} ${Math.floor(m / 100)}` }))}
      onChange={(v) => onChange(Number(v))}
      triggerClassName="dd-trigger ccm-form-select"
    />
  );
}

/** Adds a row: a value for each `by` tag. */
function AddRow({ by, options, onAdd }: { by: string[]; options: (key: string) => string[]; onAdd: (values: string[]) => void }) {
  const [picked, setPicked] = useState<string[]>([]);
  const pick = (i: number, v: string) => {
    const next = [...picked];
    next[i] = v;
    if (by.every((_, j) => next[j])) {
      onAdd(next);
      setPicked([]);
    } else setPicked(next);
  };
  return (
    <span className="ccm-add-row">
      {by.map((k, i) => (
        <Select
          key={k}
          value={picked[i] ?? ''}
          options={[{ value: '', label: `Select ${capital(k)}…` }, ...options(k).map((v) => ({ value: v, label: v }))]}
          onChange={(v) => v && pick(i, v)}
          triggerClassName="dd-trigger ccm-form-select ccm-add-select"
        />
      ))}
    </span>
  );
}

function preview(months: number[], budget: number[], actual: (number | null)[]): EChartsOption {
  const muted = token('--text-muted');
  return {
    grid: { left: 8, right: 8, top: 28, bottom: 4, containLabel: true },
    tooltip: { trigger: 'axis', confine: true, valueFormatter: (v) => (v == null ? '—' : money(Number(v), { precise: true })) },
    legend: { top: 0, left: 0, textStyle: { color: muted } },
    xAxis: { type: 'category', data: months.map(monthHead), axisLabel: { color: muted }, axisTick: { show: false } },
    yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v), color: muted } },
    series: [
      { name: 'Budget', type: 'bar', data: budget, barMaxWidth: 48, color: token('--chart-1') },
      { name: 'Actual cost', type: 'line', data: actual, symbol: 'circle', symbolSize: 6, color: token('--chart-4'), connectNulls: false },
    ],
  };
}

/** Plan → Budgets → New Budget / Edit: a budget's months, scope, tags and amounts, saved as Datadog's budget document. */
export function BudgetEditorPage() {
  const { id } = useParams();
  const existing = useQuery({ queryKey: ['cloud-cost', 'budget', id], queryFn: ({ signal }) => getBudget(id!, signal), enabled: !!id });
  if (id && existing.isPending) return <Loading label="Loading budget…" />;
  if (id && existing.isError) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;
  return <BudgetForm key={id ?? 'new'} id={id} initial={existing.data ? draftOf(existing.data) : newDraft()} created={existing.data?.attributes.created_at} />;
}

function BudgetForm({ id, initial, created }: { id?: string; initial: Draft; created?: number }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [d, setD] = useState<Draft>(initial);
  const [editingName, setEditingName] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(initial.filters.length > 0);
  const [filterKey, setFilterKey] = useState('');
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const update = (patch: Partial<Draft>) =>
    setD((prev) => {
      const next = { ...prev, ...patch };
      return next.customName ? next : { ...next, name: autoName(next) };
    });

  const now = today();
  const months = useMemo(() => monthsOf(d.start, d.end), [d.start, d.end]);
  const choices = useMemo(() => {
    const first = yyyymm(monthStart(now, -24));
    return monthsOf(first, yyyymm(monthStart(now, 36)));
  }, [now]);

  // What the scope cost: tags and values over the last six full months, and their monthly average.
  const sixMonths = useMemo(() => ({ from: monthStart(now, -6), to: monthStart(now) - 1 }), [now]);
  const providerScope = d.provider ? `providername:"${d.provider}"` : '';
  const scope = filterQuery(Object.fromEntries(d.filters.map(([k, v]) => [k, [v]])), providerScope);
  const allTags = useCostTags(sixMonths);
  const tags = useCostTags(sixMonths, scope);
  const avg = useCosts(d.by.length ? { query: scope, range: sixMonths, interval: 'month', groupBy: d.by, limit: 200 } : null);
  const avgOf = (values: string[]) => {
    const s = avg.data?.series.find((x) => d.by.every((k, i) => x.tags[k] === values[i]));
    return s ? s.total / 6 : 0;
  };
  const passed = months.filter((m) => m <= yyyymm(now));
  const actual = useCosts(
    passed.length
      ? { query: scope, range: { from: msOf(passed[0]), to: Math.min(msOf(nextMonth(passed[passed.length - 1])) - 1, now + DAY - 1) }, interval: 'month', groupBy: d.by, limit: 200 }
      : null,
  );

  const providers = (allTags.data?.find((t) => t.key === 'providername')?.values ?? []).map((v) => v.value);
  const tagKeys = (tags.data ?? []).map((t) => t.key).sort((a, b) => a.localeCompare(b));
  const valuesOf = (key: string) => tagValues(tags.data, key);

  const setAmount = (row: number, month: number, amount: number) =>
    setD((prev) => ({ ...prev, rows: prev.rows.map((r, i) => (i === row ? { ...r, amounts: { ...r.amounts, [month]: amount } } : r)) }));
  const addRow = (values: string[]) =>
    setD((prev) =>
      prev.rows.some((r) => rowKey(r.values) === rowKey(values)) ? prev : { ...prev, rows: [...prev.rows, { values, amounts: Object.fromEntries(months.map((m) => [m, Math.round(avgOf(values))])) }] },
    );
  const fill = (f: (r: Row) => (m: number) => number) => setD((prev) => ({ ...prev, rows: prev.rows.map((r) => ({ ...r, amounts: Object.fromEntries(months.map((m) => [m, f(r)(m)])) })) }));

  const total = (r: Row) => months.reduce((a, m) => a + (r.amounts[m] ?? 0), 0);
  const monthTotals = months.map((m) => d.rows.reduce((a, r) => a + (r.amounts[m] ?? 0), 0));
  const grand = monthTotals.reduce((a, v) => a + v, 0);
  const avgTotal = d.rows.reduce((a, r) => a + avgOf(r.values), 0);

  const downloadTemplate = () => {
    const rows = d.rows.length
      ? d.rows
      : valuesOf(d.by[0] ?? '')
          .slice(0, 20)
          .map((v) => ({ values: [v, ...d.by.slice(1).map(() => '')], amounts: {} as Record<number, number> }));
    const head = [...d.by, ...months.map((m) => `${Math.floor(m / 100)}-${String(m % 100).padStart(2, '0')}`)];
    const lines = [head, ...rows.map((r) => [...r.values, ...months.map((m) => r.amounts[m] ?? 0)])].map((l) => l.map(csvCell).join(','));
    const url = URL.createObjectURL(new Blob([lines.join('\n') + '\n'], { type: 'text/csv' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = 'budget-template.csv';
    a.click();
    URL.revokeObjectURL(url);
  };

  const uploadCsv = async (file: File) => {
    const rows = parseCsv(await file.text());
    const [head, ...body] = rows;
    if (!head) return;
    const cols = head.map((h) => h.trim());
    const keys = cols.slice(0, d.by.length);
    if (keys.join() !== d.by.join()) {
      setError(`The CSV's first columns must be ${d.by.join(', ')}.`);
      return;
    }
    const monthCols = cols.slice(d.by.length).map((c) => {
      const m = /^(\d{4})-?(\d{2})$/.exec(c);
      return m ? Number(m[1]) * 100 + Number(m[2]) : NaN;
    });
    setError(null);
    setD((prev) => ({
      ...prev,
      rows: body.map((r) => ({
        values: r.slice(0, d.by.length).map((v) => v.trim()),
        amounts: Object.fromEntries(monthCols.map((m, i) => [m, Number(r[d.by.length + i]) || 0]).filter(([m]) => months.includes(m as number))),
      })),
    }));
  };

  const save = useMutation({
    mutationFn: () => {
      const entries: BudgetEntry[] = d.rows.flatMap((r) =>
        months.map((m) => ({ month: m, amount: Math.max(0, r.amounts[m] ?? 0), tag_filters: d.by.map((k, i) => ({ tag_key: k, tag_value: r.values[i] })) })),
      );
      return saveBudget({
        type: 'budget',
        ...(id ? { id } : {}),
        attributes: { name: d.name.trim(), metrics_query: metricsQuery(d), start_month: d.start, end_month: d.end, entries, ...(created ? { created_at: created } : {}) },
      });
    },
    onSuccess: (doc) => {
      qc.invalidateQueries({ queryKey: ['cloud-cost', 'budgets'] });
      qc.invalidateQueries({ queryKey: ['cloud-cost', 'budget'] });
      navigate(`/cloud-cost/budgets/${doc.id}`);
    },
    onError: (e) => setError(e instanceof Error ? e.message : String(e)),
  });

  const problems = [!d.name.trim() && 'Name the budget.', !d.by.length && 'Pick what to budget by.', !d.rows.length && `Add at least one ${d.by.join(' / ') || 'row'} to budget.`].filter(
    Boolean,
  ) as string[];
  const scopeOk = budgetScope(metricsQuery(d)) != null;

  useEffect(() => {
    // Rows follow the budget's months: new months start at the 6-month average.
    setD((prev) => ({ ...prev, rows: prev.rows.map((r) => ({ ...r, amounts: Object.fromEntries(months.map((m) => [m, r.amounts[m] ?? Math.round(avgOf(r.values))])) })) }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [months]);

  const setKind = (kind: Kind) =>
    update({ kind, by: kind === 'basic' ? d.by.slice(0, 1) : d.by.length > 1 ? d.by : [...d.by, tagKeys.find((k) => !d.by.includes(k) && k === 'servicename') ?? 'servicename'], rows: [] });

  return (
    <div className="ccm-editor">
      <header className="ccm-crumbs">
        <Link to="/cloud-cost">
          <CircleDollarSign size={18} /> Cloud Cost
        </Link>
        <ChevronRight size={16} className="muted-text" />
        <Link to="/cloud-cost/budgets">Budgets</Link>
        <ChevronRight size={16} className="muted-text" />
        <span>{id ? 'Edit Budget' : 'New Budget'}</span>
      </header>
      <div className="ccm-editor-body">
        <div className="ccm-kinds">
          {(['basic', 'hierarchical'] as Kind[]).map((k) => (
            <button key={k} className={`ccm-kind${d.kind === k ? ' active' : ''}`} onClick={() => d.kind !== k && setKind(k)}>
              <span className={`ccm-kind-art ccm-kind-art-${k}`}>
                {k === 'basic' ? (
                  <>
                    <i>$30</i>
                    <i>$30</i>
                    <i>$30</i>
                  </>
                ) : (
                  <>
                    <i className="wide">$30</i>
                    <span>
                      <i>$10</i>
                      <i>$10</i>
                      <i>$10</i>
                    </span>
                  </>
                )}
              </span>
              <span>
                <b>
                  {d.kind === k && <Check size={16} />} {k === 'basic' ? 'Basic' : 'Hierarchical'}
                </b>
                <small>{k === 'basic' ? 'Create a flat, single-level budget' : 'Create a parent-child, 2-level budget'}</small>
              </span>
            </button>
          ))}
        </div>

        <div className="ccm-form">
          <label>
            Name <em>*</em>
          </label>
          <div>
            {editingName ? (
              <input
                autoFocus
                className="ccm-input ccm-name-input"
                value={d.name}
                onChange={(e) => setD((p) => ({ ...p, name: e.target.value, customName: true }))}
                onBlur={() => setEditingName(false)}
              />
            ) : (
              <span className="ccm-name">
                {d.name}
                <button className="icon-plain" title="Edit name" onClick={() => setEditingName(true)}>
                  <Pencil size={15} />
                </button>
              </span>
            )}
          </div>

          <label>Set monthly, starting</label>
          <div className="ccm-form-row">
            <MonthSelect value={d.start} months={choices} onChange={(m) => update({ start: m, end: Math.max(m, d.end) })} />
            <span>and ending on</span>
            <MonthSelect value={d.end} months={choices.filter((m) => m >= d.start && monthsOf(d.start, m).length <= 60)} onChange={(m) => update({ end: m })} />
          </div>

          <label>
            Scope to <em>*</em>
          </label>
          <div>
            <Select
              label="Provider"
              value={d.provider}
              options={[{ value: '', label: 'All Providers' }, ...providers.map((p) => ({ value: p, label: providerLabel(p) }))]}
              onChange={(p) => update({ provider: p, rows: [] })}
              triggerClassName="dd-trigger ccm-form-select"
            />
          </div>

          <label>
            Budget by <em>*</em>
          </label>
          <div className="ccm-form-row">
            {d.by.map((k, i) => (
              <span key={i} className="ccm-form-row">
                {i > 0 && <span className="muted-text">then</span>}
                <Select
                  value={k}
                  options={[...new Set([k, ...tagKeys])].filter((t) => t === k || !d.by.includes(t)).map((t) => ({ value: t, label: t }))}
                  onChange={(t) => update({ by: d.by.map((x, j) => (j === i ? t : x)), rows: [] })}
                  triggerClassName="dd-trigger ccm-form-select ccm-by-select"
                />
              </span>
            ))}
          </div>
        </div>

        <button className="ccm-optional" onClick={() => setFiltersOpen((o) => !o)}>
          {filtersOpen ? <ChevronDown size={16} /> : <ChevronRight size={16} />} Filter Cost to Budget on <span className="muted-text">(Optional)</span>
        </button>
        {filtersOpen && (
          <div className="ccm-budget-filters">
            {d.filters.map(([k, v], i) => (
              <span key={`${k}:${v}`} className="ccm-chip">
                {k}:{v}
                <button className="icon-plain" title="Remove" onClick={() => update({ filters: d.filters.filter((_, j) => j !== i), rows: [] })}>
                  <X size={12} />
                </button>
              </span>
            ))}
            <Select
              value={filterKey}
              options={[{ value: '', label: 'Tag…' }, ...tagKeys.filter((k) => !d.by.includes(k) && k !== 'providername').map((k) => ({ value: k, label: k }))]}
              onChange={setFilterKey}
              triggerClassName="dd-trigger ccm-form-select"
            />
            {filterKey && (
              <Select
                value=""
                options={[{ value: '', label: 'Value…' }, ...valuesOf(filterKey).map((v) => ({ value: v, label: v }))]}
                onChange={(v) => {
                  if (!v) return;
                  update({ filters: [...d.filters.filter(([k]) => k !== filterKey), [filterKey, v]], rows: [] });
                  setFilterKey('');
                }}
                triggerClassName="dd-trigger ccm-form-select"
              />
            )}
          </div>
        )}

        <div className="ccm-amounts-tools">
          <Menu
            trigger={<Settings size={16} />}
            triggerClassName="icon-btn"
            items={[
              { label: 'Set every month to the 6-month average', onClick: () => fill((r) => () => Math.round(avgOf(r.values))) },
              { label: `Copy ${monthHead(months[0] ?? d.start)} to every month`, onClick: () => fill((r) => () => r.amounts[months[0]] ?? 0) },
              { label: 'Clear all amounts', onClick: () => fill(() => () => 0) },
            ]}
          />
        </div>
        <div className="ccm-amounts">
          <table>
            <thead>
              <tr>
                <th className="sticky">{d.by.join(' / ').toUpperCase()}</th>
                <th className="num sticky2">6MO MONTHLY AVG</th>
                <th className="num sticky3 total">TOTAL</th>
                {months.map((m) => (
                  <th key={m} className="num">
                    {monthHead(m)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              <tr className="ccm-amounts-total">
                <td className="sticky">TOTAL</td>
                <td className="num sticky2">{d.rows.length ? money(avgTotal) : '—'}</td>
                <td className="num sticky3 total">{d.rows.length ? money(grand) : '—'}</td>
                {monthTotals.map((v, i) => (
                  <td key={months[i]} className="num">
                    {d.rows.length ? money(v) : '—'}
                  </td>
                ))}
              </tr>
              {d.rows.map((r, i) => (
                <tr key={rowKey(r.values)}>
                  <td className="sticky">
                    <span className="ccm-row-name">
                      {r.values.join(' / ')}
                      <button className="icon-plain" title="Remove" onClick={() => setD((p) => ({ ...p, rows: p.rows.filter((_, j) => j !== i) }))}>
                        <X size={13} />
                      </button>
                    </span>
                  </td>
                  <td className="num sticky2">{money(avgOf(r.values))}</td>
                  <td className="num sticky3 total">{money(total(r))}</td>
                  {months.map((m) => (
                    <td key={m} className="num">
                      <input
                        className="ccm-input ccm-amount"
                        type="number"
                        min={0}
                        step="any"
                        value={r.amounts[m] ?? 0}
                        onChange={(e) => setAmount(i, m, Number(e.target.value) || 0)}
                        aria-label={`${r.values.join(' / ')} ${monthHead(m)}`}
                      />
                    </td>
                  ))}
                </tr>
              ))}
              <tr>
                <td className="sticky" colSpan={3}>
                  {d.by.length > 0 && <AddRow by={d.by} options={valuesOf} onAdd={addRow} />}
                </td>
                <td colSpan={months.length} />
              </tr>
            </tbody>
          </table>
        </div>
        <div className="ccm-amounts-foot">
          <button className="btn btn-primary btn-sm" onClick={downloadTemplate} disabled={!d.by.length}>
            Download CSV Template <Download size={14} />
          </button>
          <button className="btn btn-sm" onClick={() => fileRef.current?.click()} disabled={!d.by.length}>
            <Upload size={14} /> Upload CSV
          </button>
          <input
            ref={fileRef}
            type="file"
            accept=".csv,text/csv"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void uploadCsv(f);
              e.target.value = '';
            }}
          />
        </div>

        <h2 className="ccm-preview-title">Preview summary</h2>
        <div className="ccm-preview">
          {d.rows.length ? (
            <>
              <div className="ccm-preview-stats">
                <span>
                  Total budget <b>{money(grand)}</b>
                </span>
                <span>
                  Monthly average <b>{money(months.length ? grand / months.length : 0)}</b>
                </span>
                <span>
                  6-month average cost <b>{money(avgTotal)}</b>
                </span>
              </div>
              <Chart
                height={260}
                option={preview(
                  months,
                  monthTotals,
                  months.map((m) => {
                    const i = actual.data?.buckets.indexOf(msOf(m)) ?? -1;
                    if (i < 0) return null;
                    // What the budgeted rows cost that month.
                    const keys = new Set(d.rows.map((r) => rowKey(r.values)));
                    return (actual.data?.series ?? []).filter((x) => keys.has(rowKey(d.by.map((k) => x.tags[k])))).reduce((sum, x) => sum + x.values[i], 0);
                  }),
                )}
              />
            </>
          ) : (
            <p className="muted-text">Add what to budget to preview the budget by month.</p>
          )}
        </div>

        {(error || (!scopeOk && d.by.length > 0)) && <div className="ccm-form-error">{error ?? 'The scope and tags do not make a valid cost query.'}</div>}
        <div className="ccm-editor-actions">
          <Link className="btn" to={id ? `/cloud-cost/budgets/${id}` : '/cloud-cost/budgets'}>
            Cancel
          </Link>
          <button className="btn btn-primary" disabled={problems.length > 0 || save.isPending} title={problems.join(' ')} onClick={() => (setError(null), save.mutate())}>
            {save.isPending ? 'Saving…' : id ? 'Save Budget' : 'Create Budget'}
          </button>
        </div>
      </div>
    </div>
  );
}

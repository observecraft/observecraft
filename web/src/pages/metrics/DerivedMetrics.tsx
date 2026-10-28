import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { LineChart, Pencil, Plus, Search, Trash2, X } from 'lucide-react';
import { createDerivedMetric, deleteDerivedMetric, listDerivedMetrics, updateDerivedMetric, type DerivedMetric } from '../../api/derivedMetrics/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useTime } from '../../components/shell/TimeContext';
import { filterDerived, queryRow } from '../../lib/derivedMetrics';
import { expandFormula } from '../../lib/metricFormula';
import { timeAgo } from '../../lib/format';
import { useMetricCatalog } from '../dashboards/queries';
import { DashboardContext, type DashboardContextValue } from '../dashboards/widgets/context';
import { TimeseriesWidget } from '../dashboards/widgets/MetricWidgets';
import { useAccess } from '../settings/queries';
import { AGGS, LETTERS, MetricPicker, toQuery, type Agg, type Row } from './MetricsExplorer';

const KEY = ['metrics', 'derived'];

/** Metrics → Derived Metrics: metric names that resolve to a formula over existing metrics, at query time. */
export function DerivedMetrics() {
  const qc = useQueryClient();
  const { canWrite } = useAccess();
  const q = useQuery({ queryKey: KEY, queryFn: listDerivedMetrics });
  const catalog = useMetricCatalog();
  const [search, setSearch] = useState('');
  const [editing, setEditing] = useState<DerivedMetric | 'new' | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['metrics'] });
  const remove = useMutation({ mutationFn: deleteDerivedMetric, onSuccess: refresh });
  const rows = useMemo(() => filterDerived(q.data ?? [], search), [q.data, search]);
  const tagsOf = (name: string) => catalog.data?.find((m) => m.name === name)?.tags ?? [];
  return (
    <div className="mx-page">
      <div className="dm-header">
        <h2 className="mx-title">Generate metrics from existing metrics</h2>
        {canWrite && (
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Metric
          </button>
        )}
      </div>
      <label className="tv-filter dm-search">
        <Search size={15} />
        <input placeholder="Search derived metrics…" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search derived metrics" />
      </label>
      {remove.error && <p className="form-error">{(remove.error as Error).message}</p>}
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="sx-results">
          <table className="sx-table dm-table">
            <thead>
              <tr>
                <th>DERIVED METRIC NAME</th>
                <th>TAGS</th>
                <th>CREATED BY</th>
                <th>LAST UPDATED</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((m) => (
                <tr key={m.id}>
                  <td>
                    <div className="dm-name">
                      <Link className="mono" to={`/metrics?metric=${encodeURIComponent(m.name)}`} title="Graph it in the Metrics Explorer">
                        {m.name}
                      </Link>
                      <code className="dm-formula">
                        {m.expression} = {m.formula}
                      </code>
                      {m.description && <span className="muted">{m.description}</span>}
                    </div>
                  </td>
                  <td>
                    <div className="dm-tags">
                      {tagsOf(m.name).map((t) => (
                        <span key={t} className="tag-chip">
                          {t}
                        </span>
                      ))}
                    </div>
                  </td>
                  <td>{m.createdBy || '—'}</td>
                  <td className="muted">{timeAgo(m.updatedAt)}</td>
                  <td className="ix-actions">
                    <Link className="icon-plain" to={`/metrics?metric=${encodeURIComponent(m.name)}`} title="Open in Metrics Explorer">
                      <LineChart size={15} />
                    </Link>
                    {canWrite && (
                      <>
                        <button className="icon-plain" title="Edit" onClick={() => setEditing(m)}>
                          <Pencil size={15} />
                        </button>
                        <button
                          className="icon-plain"
                          title="Delete"
                          onClick={() => window.confirm(`Delete derived metric "${m.name}"? Dashboards and monitors that query it stop getting data.`) && remove.mutate(m.id)}
                        >
                          <Trash2 size={15} />
                        </button>
                      </>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && (
            <div className="dm-empty">
              <Search size={64} strokeWidth={1.5} />
              No matching results found
            </div>
          )}
        </div>
      )}
      {editing && <DerivedModal metric={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </div>
  );
}

function DerivedModal({ metric, onClose, onSaved }: { metric: DerivedMetric | null; onClose: () => void; onSaved: () => void }) {
  const { range } = useTime();
  const catalog = useMetricCatalog();
  const [name, setName] = useState(metric?.name ?? '');
  const [description, setDescription] = useState(metric?.description ?? '');
  const [rows, setRows] = useState<Row[]>(() => (metric ? metric.queries.map(queryRow) : [{ agg: 'sum', metric: '', from: '', by: '' }]));
  const [expression, setExpression] = useState(metric?.expression ?? 'a');
  const update = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const queries = rows.map(toQuery);
  const formula = rows.every((r) => r.metric) ? expandFormula(expression, Object.fromEntries(queries.map((q, i) => [LETTERS[i], q]))) : null;
  const save = useMutation({
    mutationFn: () => {
      const input = { name, description, queries, expression };
      return metric ? updateDerivedMetric(metric.id, input) : createDerivedMetric(input);
    },
    onSuccess: onSaved,
  });
  const ctx = useMemo<DashboardContextValue>(
    () => ({ range, resolve: (q) => q, resolveSearch: (q) => q, canEdit: false, onEdit: () => {}, onClone: () => {}, onDelete: () => {}, onFullscreen: () => {}, onGroupLayout: () => {} }),
    [range],
  );
  // Derived metrics read real metrics only: other derived metrics aren't offered.
  const metrics = (catalog.data ?? []).filter((m) => m.kind !== 'derived');
  return (
    <Modal onClose={onClose} label={metric ? 'Edit derived metric' : 'New derived metric'} width={980} height="auto">
      <form
        className="small-modal ar-form dm-editor"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{metric ? `Edit ${metric.name}` : 'New Derived Metric'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <b>Queries</b>
        <div className="mx-rows">
          {rows.map((r, i) => (
            <div key={i} className="mx-row">
              <span className="mx-letter">{LETTERS[i]}</span>
              <select className="mx-agg" value={r.agg} onChange={(e) => update(i, { agg: e.target.value as Agg })} title="Space aggregation">
                {AGGS.map((a) => (
                  <option key={a} value={a}>
                    {a}
                  </option>
                ))}
              </select>
              <MetricPicker value={r.metric} catalog={metrics} onChange={(m) => update(i, { metric: m })} />
              <label className="mx-field">
                from
                <input placeholder="everywhere" value={r.from} onChange={(e) => update(i, { from: e.target.value })} />
              </label>
              <label className="mx-field">
                by
                <input placeholder="(everything)" value={r.by} onChange={(e) => update(i, { by: e.target.value })} />
              </label>
              {rows.length > 1 && (
                <button type="button" className="icon-plain" title="Remove query" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                  <X size={16} />
                </button>
              )}
            </div>
          ))}
          {rows.length < LETTERS.length && (
            <div className="mx-row-actions">
              <button type="button" className="btn btn-sm" onClick={() => setRows((rs) => [...rs, { agg: 'sum', metric: '', from: '', by: '' }])}>
                <Plus size={14} /> Add Query
              </button>
            </div>
          )}
        </div>
        <label className="form-label">
          Formula — the query letters, numbers, + - * / and parentheses
          <input className="mono" value={expression} onChange={(e) => setExpression(e.target.value)} placeholder="a / b * 100" required spellCheck={false} />
        </label>
        {formula ? (
          <div className="dm-preview-chart">
            <DashboardContext.Provider value={ctx}>
              <TimeseriesWidget def={{ type: 'timeseries', display: 'line', requests: [{ query: formula }] }} />
            </DashboardContext.Provider>
          </div>
        ) : (
          <p className="muted">Pick a metric for each query and write a formula over their letters to preview it.</p>
        )}
        <div className="ar-grid">
          <label className="form-label">
            Metric name
            <input className="mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="checkout.error_ratio" maxLength={200} required disabled={Boolean(metric)} />
          </label>
          <label className="form-label">
            Description
            <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What it measures" maxLength={1000} />
          </label>
        </div>
        <p className="muted lm-preview">
          Query it like any metric, e.g.{' '}
          <code>
            avg:{name || '<name>'}
            {'{env:prod}'} by {'{host}'}
          </code>
          : the filters apply to every query of the formula and its <code>by</code> replaces theirs. A derived metric can't use another one.{metric && " Its name can't change."}
        </p>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {metric ? 'Save' : 'Create Metric'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

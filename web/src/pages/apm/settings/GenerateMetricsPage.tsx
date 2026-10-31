import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ChevronDown, Cloud, Filter, ListFilter, Pencil, Plus, Search, Trash2, X, Zap } from 'lucide-react';
import type { SpanMetricGroupBy, SpansMetric, SpansMetricInput } from '../../../api/apmSettings/client';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { big, bytes } from './format';
import { useDeleteSpansMetric, useSaveSpansMetric, useSpansMetrics, useSpanVolume } from './queries';

/** APM → Settings → Generate Metrics: span-based metrics, between ingestion and retention. */
export function GenerateSpanMetricsPage() {
  const time = useTime();
  const metrics = useSpansMetrics();
  const volume = useSpanVolume(time.range);
  const remove = useDeleteSpansMetric();
  const [search, setSearch] = useState('');
  const [editing, setEditing] = useState<SpansMetric | 'new' | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const all = metrics.data ?? [];
  const rows = all.filter((m) => !search.trim() || `${m.id} ${m.query}`.toLowerCase().includes(search.trim().toLowerCase()));
  const v = volume.data;
  return (
    <div className="settings-pad aps-metrics">
      <div className="aps-title-row">
        <div>
          <h1 className="aps-h1">Generate Custom Metrics from Spans and Traces</h1>
          <p className="muted">
            <b>{all.length}</b> custom span and trace metrics
          </p>
        </div>
        <div className="aps-total">
          <span>Total Spans Ingested</span>
          <b>{v ? big(v.spansIngested) : '–'}</b>
        </div>
      </div>
      <div className="aps-search-row">
        <label className="aps-search aps-grow">
          <Search size={14} />
          <input placeholder="Search custom metrics" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> New Metric
        </button>
      </div>
      {metrics.isLoading ? (
        <Loading />
      ) : metrics.isError ? (
        <ErrorState error={metrics.error} onRetry={() => metrics.refetch()} />
      ) : (
        <div className="aps-chain">
          <div className="aps-chain-row">
            <Cloud size={18} className="aps-chain-icon" />
            <Link to="/apm/settings/ingestion">Data Ingested from Services</Link>
            <span className="aps-chain-right">
              <span className="aps-range-chip">{time.relative.short}</span> {time.relative.label}
              {v && (
                <>
                  {' '}
                  <b>{bytes(v.bytesIngested)}</b> of data over <b>{big(v.spansIngested)}</b> spans ingested
                </>
              )}
            </span>
          </div>
          <div className="aps-chain-row">
            <Zap size={18} className="aps-chain-icon aps-zap" />
            <Link to="/apm/traces">Live Search and Analytics (past 15 min)</Link>
            <span className="aps-chain-right">
              <b>{v ? big(v.spansLive) : '–'}</b> spans searchable
            </span>
          </div>
          <section className="aps-chain-section">
            <button className="aps-chain-head" onClick={() => setCollapsed((c) => !c)}>
              <ChevronDown size={16} className={collapsed ? 'rot-90' : ''} />
              <b>CUSTOM SPAN AND TRACE METRICS</b>
              <span className="muted">Computed every minute from the ingested spans, and kept like any other metric.</span>
            </button>
            {!collapsed && (
              <table className="aps-table aps-metrics-table">
                <thead>
                  <tr>
                    <th>METRIC NAME</th>
                    <th>AGGREGATION</th>
                    <th>FILTER</th>
                    <th>DIMENSIONS</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {rows.map((m) => (
                    <tr key={m.id}>
                      <td>
                        <Link
                          to={`/metrics?metric=${encodeURIComponent(m.id)}&agg=${m.aggregationType === 'count' ? 'sum' : 'avg'}`}
                          className="aps-metric-name"
                          title="Graph it in the Metrics Explorer"
                        >
                          {m.id}
                        </Link>
                      </td>
                      <td className="mono">{m.aggregationType === 'count' ? 'count' : (m.path ?? '@duration')}</td>
                      <td className="mono aps-query" title={m.query}>
                        <Filter size={12} /> {m.query}
                      </td>
                      <td>
                        <span className="aps-dims">
                          {m.groupBy.slice(0, 2).map((g) => (
                            <span key={g.path} className="aps-dim" title={g.tagName && g.tagName !== g.path ? `${g.path} as ${g.tagName}` : g.path}>
                              {g.path}
                            </span>
                          ))}
                          {m.groupBy.length > 2 && <span className="aps-dim-more">+{m.groupBy.length - 2}</span>}
                        </span>
                      </td>
                      <td className="aps-row-actions">
                        <button className="icon-plain" title="Edit" onClick={() => setEditing(m)}>
                          <Pencil size={14} />
                        </button>
                        <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete the metric ${m.id}?`) && remove.mutate(m.id)}>
                          <Trash2 size={14} />
                        </button>
                      </td>
                    </tr>
                  ))}
                  {rows.length === 0 && (
                    <tr>
                      <td colSpan={5} className="muted aps-center">
                        {all.length ? 'No metrics match.' : 'No span-based metrics yet: create one to count or measure spans over the long term.'}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            )}
          </section>
          <div className="aps-chain-row">
            <ListFilter size={18} className="aps-chain-icon" />
            <Link to="/apm/settings/retention-filters">Retention Filters</Link>
          </div>
          <div className="aps-chain-row aps-chain-nested">
            <ListFilter size={18} className="aps-chain-icon" />
            <Link to="/apm/traces">Trace Search and Analytics (past {v?.retentionDays ?? 15} days)</Link>
            <span className="aps-chain-right">
              <b>{v ? big(v.spansIndexed) : '–'}</b> spans kept by the retention filters
            </span>
          </div>
        </div>
      )}
      {editing && <MetricModal metric={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function MetricModal({ metric, onClose }: { metric: SpansMetric | null; onClose: () => void }) {
  const save = useSaveSpansMetric();
  const [form, setForm] = useState<SpansMetricInput>({
    id: metric?.id ?? '',
    query: metric?.query ?? '',
    aggregationType: metric?.aggregationType ?? 'count',
    path: metric?.path ?? '@duration',
    includePercentiles: metric?.includePercentiles ?? false,
    groupBy: metric?.groupBy ?? [],
  });
  const set = (p: Partial<SpansMetricInput>) => setForm((f) => ({ ...f, ...p }));
  const groups: SpanMetricGroupBy[] = form.groupBy ?? [];
  const setGroup = (i: number, p: Partial<SpanMetricGroupBy>) => set({ groupBy: groups.map((g, j) => (j === i ? { ...g, ...p } : g)) });
  const distribution = form.aggregationType === 'distribution';
  return (
    <Modal onClose={onClose} label={metric ? 'Edit metric' : 'New metric'} width={700} height="auto">
      <form
        className="small-modal aps-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ id: metric?.id, input: { ...form, groupBy: groups.filter((g) => g.path.trim()) } }, { onSuccess: onClose });
        }}
      >
        <div className="modal-title-row">
          <h3>{metric ? `Edit ${metric.id}` : 'Generate a Metric from Spans'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="aps-field">
          Metric name
          <input className="mono" value={form.id} disabled={!!metric} onChange={(e) => set({ id: e.target.value })} placeholder="storefront.checkout.requests" autoFocus={!metric} />
        </label>
        <label className="aps-field">
          Spans to include (span search syntax)
          <input className="mono" value={form.query} onChange={(e) => set({ query: e.target.value })} placeholder="service:storefront-api @http.status_code:2*" />
        </label>
        <div className="aps-form-grid">
          <label className="aps-field">
            Measure
            <select value={form.aggregationType} disabled={!!metric} onChange={(e) => set({ aggregationType: e.target.value })}>
              <option value="count">Count of matching spans</option>
              <option value="distribution">Distribution of a numeric attribute</option>
            </select>
          </label>
          {distribution && (
            <label className="aps-field">
              Attribute
              <input className="mono" value={form.path ?? ''} disabled={!!metric} onChange={(e) => set({ path: e.target.value })} placeholder="@duration" />
            </label>
          )}
        </div>
        {distribution && (
          <label className="aps-check">
            <input type="checkbox" checked={!!form.includePercentiles} onChange={(e) => set({ includePercentiles: e.target.checked })} /> Include percentiles (p50, p75, p90, p95, p99)
          </label>
        )}
        <div className="aps-field">
          Group by
          {groups.map((g, i) => (
            <div key={i} className="aps-group-by">
              <input className="mono" value={g.path} onChange={(e) => setGroup(i, { path: e.target.value })} placeholder="resource_name" aria-label="Path" />
              <span className="muted">as</span>
              <input className="mono" value={g.tagName ?? ''} onChange={(e) => setGroup(i, { tagName: e.target.value || null })} placeholder="tag name (optional)" aria-label="Tag name" />
              <button type="button" className="icon-plain" title="Remove" onClick={() => set({ groupBy: groups.filter((_, j) => j !== i) })}>
                <X size={14} />
              </button>
            </div>
          ))}
          <button type="button" className="link-btn aps-add-group" onClick={() => set({ groupBy: [...groups, { path: '', tagName: null }] })}>
            <Plus size={13} /> Add a dimension
          </button>
        </div>
        {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!form.id?.trim() || save.isPending}>
            {metric ? 'Save' : 'Create Metric'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

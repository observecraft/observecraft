import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { LineChart, Pencil, Plus, Trash2, X } from 'lucide-react';
import { logFields, logHistogram } from '../../api/client';
import { createLogMetric, deleteLogMetric, listLogMetrics, updateLogMetric, type LogMetric, type LogMetricAggregation, type LogMetricGroupBy, type LogMetricInput } from '../../api/logMetrics/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { compact } from '../../lib/format';
import { Sparkline } from '../errors/issueVisuals';
import { useProfile } from '../settings/queries';

const KEY = ['log-metrics'];
const MAX_GROUP_BYS = 10;
const HOUR_MS = 3_600_000;
/** Fields every log has; tags and attributes come from the recent logs. */
const RESERVED = ['service', 'status', 'host', 'source'];

/** A count is summed across series; a distribution is queried by statistic (`<name>.p95`). */
const explorerLink = (m: LogMetric) => (m.aggregation === 'count' ? `/metrics?metric=${encodeURIComponent(m.name)}&agg=sum` : `/metrics?metric=${encodeURIComponent(`${m.name}.p95`)}`);
const tagOf = (g: LogMetricGroupBy) => g.tagName || g.path.replace(/^@/, '');

/** Log Configuration → Generate Metrics: count matching logs (or measure an attribute) as metrics, computed at ingest. */
export function GenerateMetricsPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const org = me.data?.org.id;
  const canEdit = me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listLogMetrics });
  const [editing, setEditing] = useState<LogMetric | 'new' | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const remove = useMutation({ mutationFn: deleteLogMetric, onSuccess: refresh });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const metrics = q.data;
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Generate Metrics</h1>
        {canEdit && (
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Metric
          </button>
        )}
      </div>
      <p className="muted">
        Count the logs matching a search, or measure a numeric attribute, as a metric you can graph, put on dashboards and alert on. Every ingested log counts, after{' '}
        <Link to="/logs/pipelines">pipelines</Link> and before <Link to="/logs/indexes">indexes</Link>: logs an index excludes are counted too. Metrics start from the moment they are created.
      </p>
      {remove.error && <p className="form-error">{(remove.error as Error).message}</p>}
      {metrics.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No generated metrics yet.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>METRIC</th>
              <th>QUERY</th>
              <th>AGGREGATION</th>
              <th>GROUP BY</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {metrics.map((m) => (
              <tr key={m.id}>
                <td>
                  <Link className="mono" to={explorerLink(m)} title="Open in Metrics Explorer">
                    {m.name}
                  </Link>
                </td>
                <td className="mono">
                  <Link to={`/logs?query=${encodeURIComponent(m.query)}`}>{m.query || <span className="muted">all logs</span>}</Link>
                </td>
                <td>
                  {m.aggregation === 'count' ? (
                    'Count'
                  ) : (
                    <>
                      Distribution of <code>{m.path}</code>
                    </>
                  )}
                </td>
                <td>
                  {m.groupBy.length === 0 ? (
                    <span className="muted">—</span>
                  ) : (
                    m.groupBy.map((g) => (
                      <code key={g.path} className="lm-tag">
                        {tagOf(g)}
                      </code>
                    ))
                  )}
                </td>
                <td className="ix-actions">
                  <Link className="icon-plain" to={explorerLink(m)} title="Open in Metrics Explorer">
                    <LineChart size={15} />
                  </Link>
                  {canEdit && (
                    <>
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(m)}>
                        <Pencil size={15} />
                      </button>
                      <button
                        className="icon-plain"
                        title="Delete"
                        onClick={() => window.confirm(`Delete metric "${m.name}"? It stops being computed; points already written stay until they expire.`) && remove.mutate(m.id)}
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
      )}
      {editing && <MetricModal metric={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </div>
  );
}

/** `value`, once it has stopped changing for `ms`. */
function useSettled<T>(value: T, ms = 400): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/** How many logs matched the query over the last hour, with a bar per histogram bucket. */
function MatchEstimate({ query }: { query: string }) {
  const settled = useSettled(query);
  const est = useQuery({
    queryKey: ['log-metric-estimate', settled],
    queryFn: () => {
      const to = Date.now();
      return logHistogram({ range: { from: to - HOUR_MS, to }, query: settled });
    },
  });
  if (est.isPending) return <p className="muted lm-estimate">Counting matching logs…</p>;
  if (est.isError) return <p className="form-error lm-estimate">{(est.error as Error).message}</p>;
  const values = est.data.map((b) => Object.values(b.counts).reduce((a, n) => a + n, 0));
  const total = values.reduce((a, n) => a + n, 0);
  return (
    <div className="lm-estimate">
      <Sparkline values={values.length ? values : [0]} width={160} height={28} />
      <span>
        <b>{compact(total)}</b> logs matched in the last hour <span className="muted">(indexed logs; excluded ones count too once the metric exists)</span>
      </span>
    </div>
  );
}

function MetricModal({ metric, onClose, onSaved }: { metric: LogMetric | null; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(metric?.name ?? '');
  const [query, setQuery] = useState(metric?.query ?? '');
  const [aggregation, setAggregation] = useState<LogMetricAggregation>(metric?.aggregation ?? 'count');
  const [path, setPath] = useState(metric?.path ?? '');
  const [groupBy, setGroupBy] = useState<LogMetricGroupBy[]>(metric?.groupBy ?? []);
  const [adding, setAdding] = useState('');
  const fields = useQuery({
    queryKey: ['log-metric-fields'],
    queryFn: () => {
      const to = Date.now();
      return logFields({ range: { from: to - HOUR_MS, to }, query: '' });
    },
    staleTime: 60_000,
  });
  const known = fields.data?.map((f) => f.key) ?? [];
  const attributes = known.filter((k) => k.startsWith('@'));
  const input: LogMetricInput = { name, query, aggregation, path: aggregation === 'distribution' ? path : null, groupBy };
  const save = useMutation({ mutationFn: () => (metric ? updateLogMetric(metric.id, input) : createLogMetric(input)), onSuccess: onSaved });
  const addGroupBy = () => {
    const p = adding.trim();
    if (p && !groupBy.some((g) => g.path === p)) setGroupBy((gs) => [...gs, { path: p, tagName: '' }]);
    setAdding('');
  };
  const setTag = (i: number, tagName: string) => setGroupBy((gs) => gs.map((g, j) => (j === i ? { ...g, tagName } : g)));
  const by = groupBy.map(tagOf).join(',');
  const preview = `${aggregation === 'count' ? 'sum' : 'avg'}:${name || '<name>'}${aggregation === 'count' ? '' : '.p95'}{*}${by ? ` by {${by}}` : ''}`;
  return (
    <Modal onClose={onClose} label={metric ? 'Edit metric' : 'New metric'} width={720} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{metric ? `Edit ${metric.name}` : 'Generate a Metric'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Query — logs matching this search are counted
          <input className="mono" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="all logs (or e.g. service:order-service status:error)" autoFocus={!metric} />
        </label>
        <MatchEstimate query={query.trim()} />
        <div className="ar-grid">
          <label className="form-label">
            Aggregation
            <select value={aggregation} onChange={(e) => setAggregation(e.target.value as LogMetricAggregation)}>
              <option value="count">Count of matching logs</option>
              <option value="distribution">Distribution of an attribute</option>
            </select>
          </label>
          {aggregation === 'distribution' && (
            <label className="form-label">
              Measure (numeric attribute)
              <input className="mono" list="lm-attributes" value={path} onChange={(e) => setPath(e.target.value)} placeholder="@duration" required />
              <datalist id="lm-attributes">
                {attributes.map((a) => (
                  <option key={a} value={a} />
                ))}
              </datalist>
            </label>
          )}
        </div>
        <div className="ix-excl-head">
          <b>Group by</b>
          <span className="muted">
            Each value becomes a tag on the metric (up to {MAX_GROUP_BYS}); logs without it are tagged N/A. Keep values bounded: at most 1,000 series per metric are counted every 10 s.
          </span>
        </div>
        {groupBy.map((g, i) => (
          <div key={g.path} className="ix-excl-row">
            <code className="ix-excl-query">{g.path}</code>
            <span className="muted">as tag</span>
            <input className="mono ix-excl-name" value={g.tagName ?? ''} onChange={(e) => setTag(i, e.target.value)} placeholder={g.path.replace(/^@/, '')} />
            <button type="button" className="icon-plain" title="Remove" onClick={() => setGroupBy((gs) => gs.filter((_, j) => j !== i))}>
              <Trash2 size={15} />
            </button>
          </div>
        ))}
        {groupBy.length < MAX_GROUP_BYS && (
          <div className="ix-excl-row">
            <input
              className="mono ix-excl-query"
              list="lm-fields"
              value={adding}
              onChange={(e) => setAdding(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  addGroupBy();
                }
              }}
              placeholder="service, env, @http.status_code…"
            />
            <datalist id="lm-fields">
              {[...new Set([...RESERVED, ...known])].map((k) => (
                <option key={k} value={k} />
              ))}
            </datalist>
            <button type="button" className="btn btn-sm" onClick={addGroupBy} disabled={!adding.trim()}>
              <Plus size={13} /> Add
            </button>
          </div>
        )}
        <label className="form-label">
          Metric name
          <input className="mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="logs.checkout.errors" maxLength={200} required disabled={Boolean(metric)} />
        </label>
        <p className="muted lm-preview">
          Query it as <code>{preview}</code>
          {aggregation === 'distribution' && ' (also .count, .avg, .max, .p50, .p75, .p90, .p99)'}.{metric && " A metric's name can't change."}
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

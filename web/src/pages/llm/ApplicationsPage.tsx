import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { fmtLatency } from '../apm/visuals';
import { COST_NOTE, SeriesChart, StatCard } from './common';
import { fmtCost, fmtPercent, fmtTokens } from './format';
import { useLlmApp, useLlmQuery } from './LlmLayout';
import { useLlmOverview } from './queries';

/** Overview → Applications: requests, errors, latency, tokens and estimated cost per ML application. */
export function ApplicationsPage() {
  const q = useLlmQuery();
  const [app, setApp] = useLlmApp();
  const [groupBy, setGroupBy] = useState<'model' | 'ml_app'>(app ? 'model' : 'ml_app');
  const o = useLlmOverview(q, groupBy);
  if (o.isPending) return <Loading label="Loading applications…" />;
  if (o.isError) return <ErrorState error={o.error} onRetry={() => o.refetch()} />;
  const d = o.data;
  return (
    <div className="llm-page">
      <div className="llm-page-head">
        <h2>{app || 'All Applications'}</h2>
        <span className="muted">LLM calls (model spans) in the selected time range</span>
      </div>
      <div className="llm-stats">
        <StatCard label="Requests" value={d.requests.toLocaleString()} />
        <StatCard label="Error rate" value={fmtPercent(d.errorRate)} sub={`${d.errors.toLocaleString()} errors`} tone={d.errorRate > 0.05 ? 'error' : undefined} />
        <StatCard label="Latency p50" value={fmtLatency(d.p50Ms)} />
        <StatCard label="Latency p95" value={fmtLatency(d.p95Ms)} />
        <StatCard label="Input tokens" value={fmtTokens(d.inputTokens)} />
        <StatCard label="Output tokens" value={fmtTokens(d.outputTokens)} />
        <StatCard label="Total tokens" value={fmtTokens(d.totalTokens)} />
        <StatCard label="Estimated cost" value={fmtCost(d.estimatedCostUsd)} sub={<span title={COST_NOTE}>estimate · list prices</span>} tone="estimate" />
      </div>
      <div className="llm-toolbar">
        <span className="muted">Group charts by</span>
        <div className="llm-seg">
          <button className={groupBy === 'model' ? 'active' : ''} onClick={() => setGroupBy('model')}>
            Model
          </button>
          <button className={groupBy === 'ml_app' ? 'active' : ''} onClick={() => setGroupBy('ml_app')}>
            Application
          </button>
        </div>
      </div>
      <div className="llm-charts">
        <SeriesChart title="Requests" times={d.times} stepMs={d.stepMs} series={d.requestSeries} kind="bar" />
        <SeriesChart title="Tokens" times={d.times} stepMs={d.stepMs} series={d.tokenSeries} kind="bar" format={fmtTokens} />
        <SeriesChart title="Latency p95" times={d.times} stepMs={d.stepMs} series={d.latencySeries} format={fmtLatency} />
      </div>
      <section className="llm-section">
        <h3>Applications</h3>
        {d.apps.length === 0 ? (
          <p className="muted">No LLM calls in this time range.</p>
        ) : (
          <table className="sx-table llm-table">
            <thead>
              <tr>
                <th>ML APPLICATION</th>
                <th className="num">REQUESTS</th>
                <th className="num">ERRORS</th>
                <th className="num">P50</th>
                <th className="num">P95</th>
                <th className="num">INPUT TOKENS</th>
                <th className="num">OUTPUT TOKENS</th>
                <th className="num" title={COST_NOTE}>
                  EST. COST
                </th>
                <th>MODELS</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {d.apps.map((a) => (
                <tr key={a.mlApp} onClick={() => setApp(a.mlApp)} className={a.mlApp === app ? 'selected' : ''}>
                  <td>
                    <b>{a.mlApp}</b>
                  </td>
                  <td className="num">{a.requests.toLocaleString()}</td>
                  <td className="num">{a.errors ? <span className="sx-err">{a.errors.toLocaleString()}</span> : 0}</td>
                  <td className="num">{fmtLatency(a.p50Ms)}</td>
                  <td className="num">{fmtLatency(a.p95Ms)}</td>
                  <td className="num">{fmtTokens(a.inputTokens)}</td>
                  <td className="num">{fmtTokens(a.outputTokens)}</td>
                  <td className="num">{fmtCost(a.estimatedCostUsd)}</td>
                  <td className="muted">{a.models.join(', ')}</td>
                  <td>
                    <Link to={`/llm/traces?app=${encodeURIComponent(a.mlApp)}`} onClick={(e) => e.stopPropagation()}>
                      Traces
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}

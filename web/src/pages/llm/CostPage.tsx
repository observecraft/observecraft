import { useSearchParams } from 'react-router-dom';
import { Info } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { COST_NOTE, SeriesChart, StatCard } from './common';
import { fmtCost, fmtTokens, fmtUnitCost } from './format';
import { useLlmQuery } from './LlmLayout';
import { useLlmOverview } from './queries';

/** Overview → Cost: the token cost estimate by model or application over time. */
export function CostPage() {
  const [params, setParams] = useSearchParams();
  const by = params.get('by') === 'ml_app' ? 'ml_app' : 'model';
  const setBy = (v: 'model' | 'ml_app') => {
    const n = new URLSearchParams(params);
    n.set('by', v);
    setParams(n, { replace: true });
  };
  const q = useLlmQuery();
  const o = useLlmOverview(q, by);
  if (o.isPending) return <Loading label="Estimating cost…" />;
  if (o.isError) return <ErrorState error={o.error} onRetry={() => o.refetch()} />;
  const d = o.data;
  const perRequest = d.requests ? d.estimatedCostUsd / d.requests : 0;
  return (
    <div className="llm-page">
      <div className="llm-page-head">
        <h2>Cost</h2>
        <span className="llm-estimate-note">
          <Info size={14} /> {COST_NOTE}
        </span>
      </div>
      <div className="llm-stats">
        <StatCard label="Estimated cost" value={fmtCost(d.estimatedCostUsd)} sub="estimate · list prices" tone="estimate" />
        <StatCard label="Per request" value={fmtUnitCost(perRequest)} sub="estimate" />
        <StatCard label="Input tokens" value={fmtTokens(d.inputTokens)} />
        <StatCard label="Output tokens" value={fmtTokens(d.outputTokens)} />
        <StatCard label="Models" value={d.models.length} sub={d.unpricedModels.length ? `${d.unpricedModels.length} without a price` : 'all priced'} />
      </div>
      {d.unpricedModels.length > 0 && <p className="llm-warn">No list price for {d.unpricedModels.join(', ')}: their tokens are counted but not included in the estimate.</p>}
      <div className="llm-toolbar">
        <span className="muted">Break down by</span>
        <div className="llm-seg">
          <button className={by === 'model' ? 'active' : ''} onClick={() => setBy('model')}>
            Model
          </button>
          <button className={by === 'ml_app' ? 'active' : ''} onClick={() => setBy('ml_app')}>
            Application
          </button>
        </div>
      </div>
      <div className="llm-charts two">
        <SeriesChart title="Estimated cost (USD)" times={d.times} stepMs={d.stepMs} series={d.costSeries} kind="bar" format={(v) => `$${v < 1 ? v.toFixed(4) : v.toFixed(2)}`} height={220} />
        <SeriesChart title="Tokens" times={d.times} stepMs={d.stepMs} series={d.tokenSeries} kind="bar" format={fmtTokens} height={220} />
      </div>
      <section className="llm-section">
        <h3>By model</h3>
        <table className="sx-table llm-table">
          <thead>
            <tr>
              <th>MODEL</th>
              <th>PROVIDER</th>
              <th className="num">REQUESTS</th>
              <th className="num">INPUT TOKENS</th>
              <th className="num">OUTPUT TOKENS</th>
              <th className="num">EST. COST</th>
              <th className="num">SHARE</th>
            </tr>
          </thead>
          <tbody>
            {d.models.map((m) => (
              <tr key={m.model}>
                <td>
                  <b>{m.model || '(no model)'}</b>
                </td>
                <td>{m.provider}</td>
                <td className="num">{m.requests.toLocaleString()}</td>
                <td className="num">{fmtTokens(m.inputTokens)}</td>
                <td className="num">{fmtTokens(m.outputTokens)}</td>
                <td className="num">{m.estimatedCostUsd === null || m.estimatedCostUsd === undefined ? <span className="muted">no price</span> : fmtCost(m.estimatedCostUsd)}</td>
                <td className="num">{d.estimatedCostUsd && m.estimatedCostUsd ? `${((m.estimatedCostUsd / d.estimatedCostUsd) * 100).toFixed(1)}%` : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <section className="llm-section">
        <h3>By application</h3>
        <table className="sx-table llm-table">
          <thead>
            <tr>
              <th>ML APPLICATION</th>
              <th className="num">REQUESTS</th>
              <th className="num">TOKENS</th>
              <th className="num">EST. COST</th>
              <th className="num">PER REQUEST</th>
            </tr>
          </thead>
          <tbody>
            {d.apps.map((a) => (
              <tr key={a.mlApp}>
                <td>
                  <b>{a.mlApp}</b>
                </td>
                <td className="num">{a.requests.toLocaleString()}</td>
                <td className="num">{fmtTokens(a.inputTokens + a.outputTokens)}</td>
                <td className="num">{fmtCost(a.estimatedCostUsd)}</td>
                <td className="num">{a.requests ? fmtUnitCost(a.estimatedCostUsd / a.requests) : '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}

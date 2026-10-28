import { useMemo } from 'react';
import { useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Info } from 'lucide-react';
import type { LlmCheckSummary } from '../../api/llm/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { fmtStamp } from './common';
import { fmtPercent } from './format';
import { useLlmQuery } from './LlmLayout';
import { LlmTraceView } from './LlmTraceView';
import { useLlmEvaluations } from './queries';

type Outcome = 'fail' | 'pass' | 'all';

/** Investigate → Evaluations: built-in checks over the LLM calls and evaluations recorded on spans. */
export function EvaluationsPage() {
  const [params, setParams] = useSearchParams();
  const check = params.get('check');
  const outcome = (['fail', 'pass', 'all'].includes(params.get('outcome') ?? '') ? params.get('outcome') : 'fail') as Outcome;
  const set = (pairs: Record<string, string | null>) => {
    const n = new URLSearchParams(params);
    for (const [k, v] of Object.entries(pairs)) {
      if (v) n.set(k, v);
      else n.delete(k);
    }
    setParams(n, { replace: true });
  };
  const q = useLlmQuery();
  const e = useLlmEvaluations(q, check, outcome);
  if (e.isPending) return <Loading label="Evaluating LLM calls…" />;
  if (e.isError) return <ErrorState error={e.error} onRetry={() => e.refetch()} />;
  const d = e.data;
  const name = (id: string) => d.checks.find((c) => c.id === id)?.name ?? id;
  return (
    <div className="llm-page">
      <div className="llm-page-head">
        <h2>Evaluations</h2>
        <span className="llm-estimate-note">
          <Info size={14} /> Built-in checks run on the stored spans; no model is called. Checks marked <span className="llm-heuristic">heuristic</span> match keywords and patterns, so they miss
          subtle cases and can flag harmless ones. Evaluations your application records on spans (<code>gen_ai.evaluation.*</code>) are listed as{' '}
          <span className="llm-heuristic recorded">recorded</span>.
        </span>
      </div>
      <p className="muted llm-sample">
        {d.evaluated < d.totalCalls
          ? `Evaluated the newest ${d.evaluated.toLocaleString()} of ${d.totalCalls.toLocaleString()} LLM calls in this time range.`
          : `Evaluated ${d.evaluated.toLocaleString()} LLM calls in this time range.`}
      </p>
      <div className="llm-check-grid">
        {d.checks.map((c) => (
          <CheckCard key={c.id} c={c} times={d.times} stepMs={d.stepMs} active={check === c.id} onClick={() => set({ check: check === c.id ? null : c.id })} />
        ))}
      </div>
      <section className="llm-section">
        <div className="llm-toolbar">
          <h3>{check ? `${name(check)} results` : 'Results'}</h3>
          <div className="llm-seg">
            {(
              [
                ['fail', 'Failed'],
                ['pass', 'Passed'],
                ['all', 'All'],
              ] as [Outcome, string][]
            ).map(([id, label]) => (
              <button key={id} className={outcome === id ? 'active' : ''} onClick={() => set({ outcome: id === 'fail' ? null : id })}>
                {label}
              </button>
            ))}
          </div>
          {check && (
            <button className="link-btn" onClick={() => set({ check: null })}>
              All checks
            </button>
          )}
        </div>
        {d.results.length === 0 ? (
          <p className="mon-empty">No {outcome === 'all' ? '' : outcome === 'fail' ? 'failed ' : 'passed '}results in this time range.</p>
        ) : (
          <table className="sx-table llm-table">
            <thead>
              <tr>
                <th>DATE</th>
                <th>CHECK</th>
                <th>RESULT</th>
                <th>DETAIL</th>
                <th>ML APP</th>
                <th>MODEL</th>
                <th>INPUT</th>
                <th>OUTPUT</th>
              </tr>
            </thead>
            <tbody>
              {d.results.map((r) => (
                <tr key={`${r.spanId}-${r.check}`} onClick={() => set({ trace: r.traceId, span: r.spanId })} className={params.get('span') === r.spanId ? 'selected' : ''}>
                  <td className="sx-date">{fmtStamp(r.timestamp)}</td>
                  <td>{name(r.check)}</td>
                  <td>{r.passed === null || r.passed === undefined ? <span className="muted">—</span> : r.passed ? <span className="llm-pass">Pass</span> : <span className="llm-fail">Fail</span>}</td>
                  <td className="llm-preview">
                    {r.detail}
                    {r.score !== null && r.score !== undefined && <span className="muted"> (score {r.score})</span>}
                  </td>
                  <td>{r.mlApp}</td>
                  <td>{r.model}</td>
                  <td className="llm-preview" title={r.inputPreview}>
                    {r.inputPreview}
                  </td>
                  <td className="llm-preview" title={r.outputPreview}>
                    {r.outputPreview}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      {params.get('trace') && (
        <LlmTraceView traceId={params.get('trace')!} focusSpanId={params.get('span')} onClose={() => set({ trace: null, span: null })} onSelectSpan={(id) => set({ span: id })} />
      )}
    </div>
  );
}

function CheckCard({ c, times, stepMs, active, onClick }: { c: LlmCheckSummary; times: number[]; stepMs: number; active: boolean; onClick: () => void }) {
  const total = c.passed + c.failed;
  const option = useMemo(
    (): EChartsOption => ({
      grid: { left: 0, right: 0, top: 4, bottom: 0 },
      xAxis: { type: 'time', show: false },
      yAxis: { type: 'value', show: false },
      tooltip: { trigger: 'axis' },
      series: [
        { name: 'Passed', type: 'bar', stack: 'r', barMaxWidth: 6, color: '#3ec46d', data: times.map((t, i) => [t + stepMs / 2, c.passSeries[i] ?? 0]) },
        { name: 'Failed', type: 'bar', stack: 'r', barMaxWidth: 6, color: '#e5383b', data: times.map((t, i) => [t + stepMs / 2, c.failSeries[i] ?? 0]) },
      ],
    }),
    [c, times, stepMs],
  );
  return (
    <button className={`llm-check${active ? ' active' : ''}${c.failed ? ' has-fail' : ''}`} onClick={onClick} title={c.description}>
      <div className="llm-check-head">
        <b>{c.name}</b>
        {c.heuristic && <span className="llm-heuristic">heuristic</span>}
        {c.source === 'recorded' && <span className="llm-heuristic recorded">recorded</span>}
      </div>
      <div className="llm-check-counts">
        <span className="llm-fail">{c.failed.toLocaleString()} failed</span>
        <span className="llm-pass">{c.passed.toLocaleString()} passed</span>
        {total > 0 && <span className="muted">{fmtPercent(c.failed / total)} fail</span>}
        {c.unlabeled > 0 && <span className="muted">{c.unlabeled} unlabeled</span>}
      </div>
      {total > 0 ? <Chart option={option} height={44} /> : <p className="muted llm-check-none">Not applicable to any call yet.</p>}
      <p className="muted llm-check-desc">{c.description}</p>
    </button>
  );
}

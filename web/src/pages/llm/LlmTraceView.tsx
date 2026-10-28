import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link } from 'react-router-dom';
import { AlertTriangle, CheckCircle2, ChevronRight, Clock, ExternalLink, Maximize2, Minimize2, X, XCircle } from 'lucide-react';
import type { TraceSpan } from '../../api/apm/client';
import type { LlmMessage, LlmSpanDetail, LlmTraceDetail } from '../../api/llm/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { buildTree, Waterfall } from '../apm/trace/TraceView';
import { fmtLatency } from '../apm/visuals';
import { KindBadge } from './common';
import { fmtCost, fmtTokens } from './format';
import { useLlmTrace } from './queries';
import '../apm/explorer/explorer.css';

const KIND_COLOR: Record<string, string> = { llm: '#7c5cff', embedding: '#a78bfa', retrieval: '#14b8a6', tool: '#f59e0b', agent: '#3b82f6', workflow: '#0ea5e9', task: '#64748b' };

const CHECK_NAMES: Record<string, string> = {
  failure_to_answer: 'Failure to answer',
  error: 'Error',
  prompt_injection: 'Prompt injection',
  pii_in_output: 'PII in output',
  token_outlier: 'Token outlier',
  latency_outlier: 'Latency outlier',
};
const HEURISTIC = new Set(['failure_to_answer', 'prompt_injection', 'pii_in_output']);

/** Side panel for one trace with LLM spans: the span tree and the selected span's messages. */
export function LlmTraceView({ traceId, focusSpanId, onClose, onSelectSpan }: { traceId: string; focusSpanId: string | null; onClose: () => void; onSelectSpan: (id: string) => void }) {
  const q = useLlmTrace(traceId);
  const [full, setFull] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return createPortal(
    <div className="tv-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className={`tv llm-tv${full ? ' full' : ''}`} role="dialog" aria-label={`LLM trace ${traceId}`}>
        {q.isPending ? (
          <Loading label="Loading trace…" />
        ) : q.isError ? (
          <>
            <button className="icon-plain tv-close-alone" onClick={onClose} title="Close (Esc)">
              <X size={20} />
            </button>
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          </>
        ) : (
          <Body detail={q.data} focusSpanId={focusSpanId} onSelectSpan={onSelectSpan} onClose={onClose} full={full} setFull={setFull} />
        )}
      </aside>
    </div>,
    document.body,
  );
}

function Body({
  detail,
  focusSpanId,
  onSelectSpan,
  onClose,
  full,
  setFull,
}: {
  detail: LlmTraceDetail;
  focusSpanId: string | null;
  onSelectSpan: (id: string) => void;
  onClose: () => void;
  full: boolean;
  setFull: (f: boolean) => void;
}) {
  const { trace, llm } = detail;
  const roots = useMemo(() => buildTree(trace.spans), [trace.spans]);
  const llmSpans = trace.spans.filter((s) => llm[s.spanId]);
  const top = [...llmSpans].sort((a, b) => a.startUs - b.startUs || b.durationUs - a.durationUs)[0] ?? trace.spans[0];
  const [selected, setSelected] = useState(focusSpanId && trace.spans.some((s) => s.spanId === focusSpanId) ? focusSpanId : top.spanId);
  const [llmOnly, setLlmOnly] = useState(false);
  const span = trace.spans.find((s) => s.spanId === selected) ?? top;
  const calls = llmSpans.map((s) => llm[s.spanId]).filter((i) => i.kind === 'llm' || i.kind === 'embedding');
  const tokens = calls.reduce((n, i) => n + i.inputTokens + i.outputTokens, 0);
  const costs = calls.map((i) => i.estimatedCostUsd);
  const cost = costs.some((c) => c === null || c === undefined) ? null : costs.reduce<number>((n, c) => n + (c ?? 0), 0);
  const errors = llmSpans.filter((s) => s.status === 'Error').length;
  const matches = llmOnly ? new Set(llmSpans.map((s) => s.spanId)) : null;
  const select = (id: string) => {
    setSelected(id);
    onSelectSpan(id);
  };
  return (
    <div className="tv-body llm-tv-body">
      <header className={`tv-head${errors ? ' err' : ''}`}>
        <div className="tv-title">
          <b>{llm[top.spanId]?.mlApp ?? top.service}</b>
          <ChevronRight size={16} className="muted" />
          <span className="tv-resource" title={top.name}>
            {top.name}
          </span>
          <ChevronRight size={16} className="muted" />
          <span className="tv-tid">
            trace_id <code>{trace.traceId}</code>
          </span>
          <span className="tv-actions">
            <Link className="btn btn-sm" to={`/apm/traces?trace=${trace.traceId}&span=${span.spanId}`} title="Open this trace in APM">
              <ExternalLink size={14} /> APM
            </Link>
            <button className="icon-plain" title={full ? 'Exit full width' : 'Full width'} onClick={() => setFull(!full)}>
              {full ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
            </button>
            <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
              <X size={20} />
            </button>
          </span>
        </div>
        <div className="tv-chips">
          <span className="tv-dur">
            <Clock size={15} /> {fmtLatency(trace.durationMs)}
          </span>
          <span className="llm-chip">{llmSpans.length} LLM spans</span>
          <span className="llm-chip">{fmtTokens(tokens)} tokens</span>
          <span className="llm-chip" title="Estimate from list prices">
            {fmtCost(cost)} est.
          </span>
          {errors > 0 && <span className="sx-code-badge err">{errors} ERROR</span>}
          <span className="muted">{new Date(trace.start).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })}</span>
        </div>
      </header>
      <div className="llm-tv-split">
        <div className="llm-tv-tree">
          <label className="tv-errors llm-tv-filter">
            <input type="checkbox" className="checkbox" checked={llmOnly} onChange={() => setLlmOnly((v) => !v)} /> Highlight LLM spans only
          </label>
          <Waterfall roots={roots} total={trace.durationMs * 1000} selected={span.spanId} onSelect={select} matches={matches} color={(s) => KIND_COLOR[llm[s.spanId]?.kind ?? ''] ?? '#9aa3b2'} />
        </div>
        <SpanDetail span={span} info={llm[span.spanId]} />
      </div>
    </div>
  );
}

function SpanDetail({ span, info }: { span: TraceSpan; info: LlmSpanDetail | undefined }) {
  return (
    <section className="llm-span">
      <header>
        {info && <KindBadge kind={info.kind} />}
        <b className="llm-span-name">{span.name}</b>
        <span className="muted">{span.service}</span>
      </header>
      <dl className="llm-span-facts">
        <div>
          <dt>Duration</dt>
          <dd>{fmtLatency(span.durationUs / 1000)}</dd>
        </div>
        {info?.model && (
          <div>
            <dt>Model</dt>
            <dd>{info.model}</dd>
          </div>
        )}
        {info?.provider && (
          <div>
            <dt>Provider</dt>
            <dd>{info.provider}</dd>
          </div>
        )}
        {info && info.inputTokens + info.outputTokens > 0 && (
          <div>
            <dt>Tokens</dt>
            <dd>
              {fmtTokens(info.inputTokens)} in · {fmtTokens(info.outputTokens)} out
            </dd>
          </div>
        )}
        {info && info.inputTokens + info.outputTokens > 0 && (
          <div>
            <dt>Est. cost</dt>
            <dd title="Estimate from list prices">{fmtCost(info.estimatedCostUsd)}</dd>
          </div>
        )}
      </dl>
      {span.status === 'Error' && (
        <div className="llm-span-error">
          <AlertTriangle size={15} /> {span.statusMessage || 'Error'}
        </div>
      )}
      {!info ? (
        <p className="muted">This span carries no LLM attributes. Open the trace in APM for its details.</p>
      ) : (
        <>
          <Messages title="Input" messages={info.input} />
          <Messages title="Output" messages={info.output} empty={span.status === 'Error' ? 'No output: the call failed.' : 'No output recorded.'} />
          {(info.checks.length > 0 || info.evaluations.length > 0) && (
            <div className="llm-evals">
              <h4>Evaluations</h4>
              {info.checks.map((c) => (
                <div key={c.check} className={`llm-eval ${c.passed ? 'pass' : 'fail'}`}>
                  {c.passed ? <CheckCircle2 size={15} /> : <XCircle size={15} />}
                  <span>{CHECK_NAMES[c.check] ?? c.check}</span>
                  {HEURISTIC.has(c.check) && <span className="llm-heuristic">heuristic</span>}
                  {c.detail && <span className="muted">{c.detail}</span>}
                </div>
              ))}
              {info.evaluations.map((e) => (
                <div key={e.name} className={`llm-eval ${e.passed === false ? 'fail' : e.passed ? 'pass' : ''}`}>
                  {e.passed === false ? <XCircle size={15} /> : <CheckCircle2 size={15} />}
                  <span>{e.name}</span>
                  <span className="llm-heuristic recorded">recorded</span>
                  {e.score !== null && e.score !== undefined && <span>score {e.score}</span>}
                  {e.label && <span className="muted">{e.label}</span>}
                  {e.explanation && <span className="muted">{e.explanation}</span>}
                </div>
              ))}
            </div>
          )}
        </>
      )}
      <details className="llm-attrs">
        <summary>Span attributes ({Object.keys(span.attributes).length})</summary>
        <table>
          <tbody>
            {Object.entries(span.attributes)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([k, v]) => (
                <tr key={k}>
                  <td>{k}</td>
                  <td>{v}</td>
                </tr>
              ))}
          </tbody>
        </table>
      </details>
    </section>
  );
}

function Messages({ title, messages, empty = 'Nothing recorded.' }: { title: string; messages: LlmMessage[]; empty?: string }) {
  return (
    <div className="llm-messages">
      <h4>{title}</h4>
      {messages.length === 0 ? (
        <p className="muted">{empty}</p>
      ) : (
        messages.map((m, i) => (
          <div key={i} className={`llm-msg r-${m.role}`}>
            <span className="llm-role">{m.role}</span>
            <div className="llm-msg-text">{m.content}</div>
          </div>
        ))
      )}
    </div>
  );
}

import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import {
  AlertTriangle,
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ChevronDown,
  ChevronRight,
  Columns3,
  Copy,
  Download,
  ExternalLink,
  Filter,
  Globe,
  Image,
  Search,
  ShieldCheck,
  Sparkles,
  Wrench,
  X,
} from 'lucide-react';
import type { AiGuardEvaluationDetail, AiGuardEvaluationRow, AiGuardMessage, AiGuardQuery, AiGuardStep } from '../../../api/aiGuard/client';
import { Chart } from '../../../components/charts/Chart';
import { TimePicker } from '../../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { token } from '../../../theme/chartTheme';
import { Assessment, CategoryChip } from './AiGuardLayout';
import { useAiGuardEvaluation, useAiGuardEvaluations, useAiGuardSummary } from './queries';

const MIN = 60_000;
const fmtTime = (t: number) => new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', second: '2-digit' });
const fmtDur = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(2).replace(/\.?0+$/, '')}s` : `${Math.round(ms)}ms`);

/** A select with its label above the value. */
function Labeled({ label, value, options, onChange }: { label: string; value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <label className="aig-select">
      <span>{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">All</option>
        {options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

/** AI Guard → Investigate: every evaluated step of the AI services, with the summary cards and charts. */
export function AiGuardInvestigatePage() {
  return (
    <TimeProvider initial="1w">
      <Investigate />
    </TimeProvider>
  );
}

function Investigate() {
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: k !== 'eval' },
    );
  const [searching, setSearching] = useState(!!params.get('q'));
  const [more, setMore] = useState(!!(params.get('service') || params.get('env') || params.get('type') || params.get('category')));
  const [draft, setDraft] = useState(params.get('q') ?? '');
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const blocked = params.get('blocked');
  const q: AiGuardQuery = {
    range: r,
    query: params.get('q') ?? '',
    assessment: params.get('assessment') || undefined,
    blocked: blocked === 'true' ? true : blocked === 'false' ? false : undefined,
    service: params.get('service') || undefined,
    env: params.get('env') || undefined,
    target: params.get('type') || undefined,
    category: params.get('category') || undefined,
  };
  const summary = useAiGuardSummary(q);
  const page = useAiGuardEvaluations({ ...q, limit: 200 });
  const s = summary.data;
  const extra = ['service', 'env', 'type', 'category'].filter((k) => params.get(k)).length;

  const assessmentChart = useMemo<EChartsOption>(() => {
    const pts = s?.series ?? [];
    const bar = (name: string, color: string, data: number[]) => ({ name, type: 'bar' as const, stack: 'a', data: pts.map((p, i) => [p.t, data[i]]), itemStyle: { color }, barMaxWidth: 12 });
    return {
      grid: { left: 36, right: 8, top: 10, bottom: 40 },
      tooltip: { trigger: 'axis' },
      legend: { bottom: 0, icon: 'rect', itemWidth: 10, itemHeight: 10 },
      xAxis: { type: 'time' },
      yAxis: { type: 'value', splitNumber: 2 },
      series: [
        bar(
          'Safe',
          token('--status-ok') || '#3fa45b',
          pts.map((p) => p.safe),
        ),
        bar(
          'Unsafe',
          token('--status-error') || '#d9363e',
          pts.map((p) => p.unsafeCount),
        ),
        bar(
          'Blocked',
          token('--accent') || '#3d7bd9',
          pts.map((p) => p.blocked),
        ),
      ],
    };
  }, [s]);
  const durationChart = useMemo<EChartsOption>(() => {
    const pts = (s?.series ?? []).filter((p) => p.safe + p.unsafeCount + p.blocked > 0);
    const line = (name: string, key: 'avg' | 'p75' | 'p95' | 'p99', color: string) => ({
      name,
      type: 'line' as const,
      showSymbol: false,
      data: pts.map((p) => [p.t, p[key]]),
      lineStyle: { width: 1.5 },
      itemStyle: { color },
    });
    return {
      grid: { left: 40, right: 8, top: 10, bottom: 40 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => fmtDur(Number(v)) },
      legend: { bottom: 0, itemWidth: 10, itemHeight: 10 },
      xAxis: { type: 'time' },
      yAxis: { type: 'value', splitNumber: 2, axisLabel: { formatter: (v: number) => fmtDur(v) } },
      series: [line('Average', 'avg', '#3d7bd9'), line('p75', 'p75', '#8fb4ec'), line('p95', 'p95', '#e8a33d'), line('p99', 'p99', '#9b7fd6')],
    };
  }, [s]);

  const exportCsv = () => {
    const rows = page.data?.rows ?? [];
    const head = ['timestamp', 'type', 'direction', 'content', 'assessment', 'blocked', 'attack_categories', 'sensitive_data', 'service', 'env', 'duration_ms'];
    const cell = (v: unknown) => `"${String(v).replace(/"/g, '""')}"`;
    const csv = [
      head.join(','),
      ...rows.map((x) =>
        [new Date(x.timestamp).toISOString(), x.target, x.direction, x.content, x.action, !!x.blocked, x.tags.join(' '), x.sdsTags.join(' '), x.service, x.env, x.durationMs].map(cell).join(','),
      ),
    ].join('\n');
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    a.download = 'ai-guard-spans.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };

  return (
    <div className="sec-page aig-page">
      <div className="aig-filters">
        <div className="aig-seg">
          <span className="aig-seg-label">Search</span>
          <button className={!searching ? 'active' : ''} onClick={() => setSearching(false)} title="Filters" aria-label="Filters">
            <Filter size={14} />
          </button>
          <button className={searching ? 'active' : ''} onClick={() => setSearching(true)} title="Search" aria-label="Search">
            <Search size={14} />
          </button>
        </div>
        {searching ? (
          <form
            className="sec-search aig-grow"
            onSubmit={(e) => {
              e.preventDefault();
              set('q', draft.trim() || null);
            }}
          >
            <Search size={14} />
            <input
              placeholder="Search content, or @ai_guard.attack_categories:… @ai_guard.tool_name:… service:…"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => set('q', draft.trim() || null)}
            />
          </form>
        ) : (
          <>
            <Labeled
              label="Assessment"
              value={params.get('assessment') ?? ''}
              options={[
                ['safe', 'Safe'],
                ['unsafe', 'Unsafe'],
              ]}
              onChange={(v) => set('assessment', v || null)}
            />
            <Labeled
              label="Blocked"
              value={blocked ?? ''}
              options={[
                ['true', 'Blocked'],
                ['false', 'Not blocked'],
              ]}
              onChange={(v) => set('blocked', v || null)}
            />
            {more && (
              <>
                <Labeled label="Service" value={params.get('service') ?? ''} options={(s?.services ?? []).map((x) => [x, x])} onChange={(v) => set('service', v || null)} />
                <Labeled label="Env" value={params.get('env') ?? ''} options={(s?.envs ?? []).map((x) => [x, x])} onChange={(v) => set('env', v || null)} />
                <Labeled
                  label="Type"
                  value={params.get('type') ?? ''}
                  options={[
                    ['prompt', 'LLM'],
                    ['tool', 'Tool'],
                  ]}
                  onChange={(v) => set('type', v || null)}
                />
                <Labeled label="Attack category" value={params.get('category') ?? ''} options={(s?.categories ?? []).map((x) => [x, x])} onChange={(v) => set('category', v || null)} />
              </>
            )}
            <button className="btn aig-more" onClick={() => setMore(!more)} title={more ? 'Fewer filters' : 'More filters'}>
              {more ? '−' : `+${Math.max(extra, 4)}`}
            </button>
          </>
        )}
        <span className="aig-grow" />
        <TimePicker compact />
      </div>

      {summary.isError ? (
        <ErrorState error={summary.error} />
      ) : (
        <div className="aig-summary">
          <div className="aig-stack">
            <button
              className={`aig-card aig-card-unsafe${params.get('assessment') === 'unsafe' && blocked === 'false' ? ' on' : ''}`}
              onClick={() => setParams((p) => ({ ...Object.fromEntries(p), assessment: 'unsafe', blocked: 'false' }))}
            >
              <span className="aig-card-title">
                <AlertTriangle size={14} /> Unsafe
              </span>
              <span className="aig-card-value">
                <b>{s ? compact(s.unsafeNotBlocked) : '–'}</b> not blocked
              </span>
            </button>
            <button className="aig-card" onClick={() => set('q', '@ai_guard.sds:true')}>
              <span className="aig-card-title aig-muted">
                <ShieldCheck size={14} /> Sensitive Data
              </span>
              <span className="aig-card-value aig-plain">
                <b>{s ? compact(s.sensitive) : '–'}</b> found
              </span>
            </button>
          </div>
          <div className="aig-chart-card">
            <h4>AI Guard Assessment</h4>
            {summary.isLoading ? <Loading /> : <Chart option={assessmentChart} height={150} />}
          </div>
          <div className="aig-chart-card">
            <h4>Span Duration</h4>
            {summary.isLoading ? <Loading /> : <Chart option={durationChart} height={150} />}
          </div>
        </div>
      )}

      <div className="aig-table-card">
        <div className="aig-table-head">
          <b>{page.data ? compact(page.data.total) : '–'} spans</b>
          <span className="aig-grow" />
          <button className="btn btn-ghost btn-icon" onClick={exportCsv} title="Export as CSV" aria-label="Export as CSV">
            <Download size={14} />
          </button>
          <button className="btn btn-ghost btn-icon" title="Columns" aria-label="Columns" disabled>
            <Columns3 size={14} />
          </button>
        </div>
        {page.isLoading ? (
          <Loading />
        ) : page.isError ? (
          <ErrorState error={page.error} />
        ) : !page.data?.rows.length ? (
          <Empty>
            <b>No evaluations.</b>
            <p className="muted">
              Evaluations appear when an application calls AI Guard through the tracer's AI Guard SDK (<code>DD_AI_GUARD_ENDPOINT</code>), or from the{' '}
              <Link to="/security/ai-guard/playground">Playground</Link>.
            </p>
          </Empty>
        ) : (
          <div className="aig-scroll">
            <table className="mon-table syn-table aig-table">
              <thead>
                <tr>
                  <th>TIMESTAMP</th>
                  <th>TYPE</th>
                  <th className="aig-content-th">CONTENT</th>
                  <th>ASSESSMENT</th>
                  <th>ATTACK CATEGORIES</th>
                  <th>SENSITIVE DATA</th>
                  <th>SERVICE</th>
                  <th>ENV</th>
                  <th className="num">DURATION</th>
                </tr>
              </thead>
              <tbody>
                {page.data.rows.map((x) => (
                  <Row key={x.id} x={x} active={params.get('eval') === x.id} onOpen={() => set('eval', x.id)} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      {params.get('eval') && <SpanPanel id={params.get('eval')!} rows={page.data?.rows ?? []} onSelect={(id) => set('eval', id)} onClose={() => set('eval', null)} />}
    </div>
  );
}

export function TypeChip({ target }: { target: string }) {
  return target === 'tool' ? (
    <span className="aig-type">
      <Wrench size={11} /> Tool
    </span>
  ) : (
    <span className="aig-type">
      <Sparkles size={11} /> LLM
    </span>
  );
}

export function DirectionChip({ direction }: { direction: string }) {
  return direction === 'output' ? (
    <span className="aig-dir">
      <ArrowLeft size={11} /> Output
    </span>
  ) : (
    <span className="aig-dir">
      <ArrowRight size={11} /> Input
    </span>
  );
}

export function ServiceChip({ service }: { service: string }) {
  return (
    <span className="aig-service">
      <i>
        <Globe size={11} />
      </i>
      {service}
    </span>
  );
}

function Row({ x, active, onOpen }: { x: AiGuardEvaluationRow; active: boolean; onOpen: () => void }) {
  return (
    <tr className={`aig-row${active ? ' active' : ''}`} onClick={onOpen}>
      <td className="muted nowrap">{fmtTime(x.timestamp)}</td>
      <td>
        <TypeChip target={x.target} />
      </td>
      <td className="aig-content">
        <DirectionChip direction={x.direction} /> <span className="mono">{x.content}</span>
      </td>
      <td>
        <Assessment action={x.action} blocked={!!x.blocked} />
      </td>
      <td>
        <div className="aig-cats">{x.tags.length ? x.tags.map((t) => <CategoryChip key={t} category={t} />) : <span className="muted">—</span>}</div>
      </td>
      <td>
        <div className="aig-cats">
          {x.sdsTags.length ? (
            [...new Set(x.sdsTags)].map((t) => (
              <span key={t} className="aig-sds">
                {t}
              </span>
            ))
          ) : (
            <span className="muted">—</span>
          )}
        </div>
      </td>
      <td>
        <ServiceChip service={x.service} />
      </td>
      <td>
        <span className="aig-env">{x.env || '—'}</span>
      </td>
      <td className="num mono">{fmtDur(x.durationMs)}</td>
    </tr>
  );
}

// ---------------------------------------------------------------- span panel

const asMessage = (v: unknown) => v as AiGuardMessage;

function textOf(m: AiGuardMessage): string {
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content))
    return m.content
      .map((p) => p.text ?? '')
      .filter(Boolean)
      .join('\n');
  return '';
}

function images(m: AiGuardMessage): string[] {
  return Array.isArray(m.content) ? m.content.flatMap((p) => (p.image_url?.url ? [p.image_url.url] : [])) : [];
}

function MessageBody({ m }: { m: AiGuardMessage }) {
  const [shown, setShown] = useState<Set<string>>(new Set());
  const body = m.tool_calls?.length ? m.tool_calls.map((c) => `${c.function.name}(${prettyArgs(c.function.arguments)})`).join('\n') : textOf(m);
  return (
    <>
      <div className="aig-msg-body">
        <pre>{body || <span className="muted">(empty)</span>}</pre>
        <button className="btn btn-ghost btn-icon aig-copy" onClick={() => navigator.clipboard?.writeText(body)} title="Copy" aria-label="Copy">
          <Copy size={13} />
        </button>
      </div>
      {images(m).map((url) => (
        <div key={url} className="aig-image">
          <Image size={13} /> Image
          <button className="link" onClick={() => setShown((s) => new Set(s).add(url))}>
            Show Image
          </button>
          <button className="link" onClick={() => navigator.clipboard?.writeText(url)}>
            Copy URL
          </button>
          {shown.has(url) && <img src={url} alt="" referrerPolicy="no-referrer" />}
        </div>
      ))}
    </>
  );
}

function prettyArgs(args: string): string {
  try {
    return JSON.stringify(JSON.parse(args), null, 2);
  } catch {
    return args;
  }
}

function StepCard({ step, start, selected, onSelect, onPlayground }: { step: AiGuardStep; start: number; selected: boolean; onSelect: () => void; onPlayground: () => void }) {
  const m = asMessage(step.message);
  return (
    <div className={`aig-step${selected ? ' selected' : ''}`} onClick={onSelect}>
      <div className="aig-step-head">
        <DirectionChip direction={step.direction} />
        <span className="aig-step-kind">
          {step.target === 'tool' ? (
            <>
              <Wrench size={12} /> Tool <b>{step.toolName || 'tool'}</b>
            </>
          ) : (
            <>
              <Sparkles size={12} /> LLM <b>{m.role === 'assistant' ? 'Assistant' : m.role === 'user' ? 'User' : m.role}</b> prompt
            </>
          )}
        </span>
        <span className="muted">+{((step.timestamp - start) / 1000).toFixed(1)}s</span>
        <span className="aig-grow" />
        <Assessment action={step.action} blocked={step.blocked} />
      </div>
      <MessageBody m={m} />
      <div className="aig-step-foot">
        {step.tags.length ? (
          <span className="aig-cat-count">
            {step.tags.length} Attack Categor{step.tags.length === 1 ? 'y' : 'ies'}
          </span>
        ) : (
          <span className="muted">No rule match</span>
        )}
        <span className="aig-grow" />
        <button
          className="link"
          onClick={(e) => {
            e.stopPropagation();
            onPlayground();
          }}
        >
          Test in Playground
        </button>
      </div>
    </div>
  );
}

function Confidence({ value, flagged }: { value: number; flagged: boolean }) {
  const filled = Math.max(1, Math.round(value * 5));
  return (
    <span className="aig-conf" title={`${(value * 100).toFixed(1)}%`}>
      {Array.from({ length: 5 }, (_, i) => (
        <i key={i} className={i < filled ? (value >= 0.85 ? 'hi' : flagged ? 'mid' : 'lo') : ''} />
      ))}
    </span>
  );
}

function SpanPanel({ id, rows, onSelect, onClose }: { id: string; rows: AiGuardEvaluationRow[]; onSelect: (id: string) => void; onClose: () => void }) {
  const q = useAiGuardEvaluation(id);
  const navigate = useNavigate();
  const [contextOpen, setContextOpen] = useState(false);
  const [allTags, setAllTags] = useState(false);
  const d = q.data;
  const idx = rows.findIndex((r) => r.id === id);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const toPlayground = (messages: unknown) => navigate('/security/ai-guard/playground', { state: { messages, service: d?.service } });
  const stepIdx = d ? d.steps.findIndex((s) => s.id === d.id) : -1;
  return createPortal(
    <div className="sec-panel-backdrop" onClick={onClose}>
      <aside className="sec-panel aig-panel" onClick={(e) => e.stopPropagation()}>
        {q.isLoading ? (
          <Loading />
        ) : q.isError || !d ? (
          <ErrorState error={q.error} />
        ) : (
          <>
            <div className="aig-panel-head">
              <ServiceChip service={d.service} />
              <span className={`aig-mode${d.blockingEnabled ? ' blocking' : ''}`}>{d.blockingEnabled ? 'BLOCKING' : 'MONITORING'}</span>
              <span className="aig-grow" />
              <Link className="aig-head-link" to={d.traceId ? `/apm/traces?trace=${d.traceId}${d.spanId ? `&span=${d.spanId}` : ''}` : `/apm/traces?q=${encodeURIComponent(`service:${d.service}`)}`}>
                View in APM <ExternalLink size={12} />
              </Link>
              <Link className="aig-head-link" to={`/llm/traces?q=${encodeURIComponent(`service:${d.service}`)}`}>
                View in LLM Obs <ExternalLink size={12} />
              </Link>
              <button className="btn btn-ghost btn-icon" onClick={() => idx > 0 && onSelect(rows[idx - 1].id)} disabled={idx <= 0} aria-label="Previous span">
                <ArrowUp size={15} />
              </button>
              <button
                className="btn btn-ghost btn-icon"
                onClick={() => idx >= 0 && idx < rows.length - 1 && onSelect(rows[idx + 1].id)}
                disabled={idx < 0 || idx >= rows.length - 1}
                aria-label="Next span"
              >
                <ArrowDown size={15} />
              </button>
              <button className="btn btn-ghost btn-icon" onClick={onClose} aria-label="Close">
                <X size={16} />
              </button>
            </div>
            <div className="aig-chips">
              {d.clientIp && <span className="aig-chip mono">{d.clientIp}</span>}
              {(d.userEmail || d.userId) && (
                <span className="aig-chip">
                  {d.userEmail} {d.userId && <span className="muted">{d.userId}</span>}
                </span>
              )}
              {d.env && <span className="aig-chip">env:{d.env}</span>}
              <span className="aig-chip">duration:{fmtDur(d.durationMs)}</span>
            </div>
            <div className="aig-panel-body">
              <div className="aig-timeline">
                {d.context.length > 0 && (
                  <div className="aig-context">
                    <button className="aig-context-toggle" onClick={() => setContextOpen(!contextOpen)}>
                      Context <span className="muted">{d.context.length} messages</span> {contextOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                    </button>
                    {contextOpen &&
                      d.context.map((m, i) => (
                        <div key={i} className="aig-context-msg">
                          <span className="aig-role">{asMessage(m).role}</span>
                          <MessageBody m={asMessage(m)} />
                        </div>
                      ))}
                  </div>
                )}
                {d.steps.map((s) => (
                  <StepCard
                    key={s.id}
                    step={s}
                    start={d.steps[0].timestamp}
                    selected={s.id === d.id}
                    onSelect={() => onSelect(s.id)}
                    onPlayground={() => toPlayground([...d.context, ...d.steps.slice(0, d.steps.indexOf(s) + 1).map((x) => x.message)])}
                  />
                ))}
                <div className="aig-step-nav">
                  <button className="btn" disabled={stepIdx <= 0} onClick={() => onSelect(d.steps[stepIdx - 1].id)}>
                    <ArrowUp size={13} /> Previous step
                  </button>
                  <button className="btn" disabled={stepIdx < 0 || stepIdx >= d.steps.length - 1} onClick={() => onSelect(d.steps[stepIdx + 1].id)}>
                    <ArrowDown size={13} /> Next step
                  </button>
                </div>
              </div>
              <EvaluationSide d={d} allTags={allTags} setAllTags={setAllTags} />
            </div>
          </>
        )}
      </aside>
    </div>,
    document.body,
  );
}

function EvaluationSide({ d, allTags, setAllTags }: { d: AiGuardEvaluationDetail; allTags: boolean; setAllTags: (v: boolean) => void }) {
  const unsafe = d.action !== 'ALLOW';
  const blocked = d.blocked === 1;
  const flagged = d.scores.filter((s) => s.flagged);
  const sds = d.sdsFindings as unknown as { category: string; rule_display_name: string; rule_tag: string; location: { path: string } }[];
  const tags = [
    `service:${d.service}`,
    `env:${d.env}`,
    `ai_guard.action:${d.action}`,
    `ai_guard.target:${d.target}`,
    `ai_guard.direction:${d.direction}`,
    ...(d.toolName ? [`ai_guard.tool_name:${d.toolName}`] : []),
    `ai_guard.blocked:${blocked}`,
    `ai_guard.source:${d.source}`,
    ...(d.language ? [`language:${d.language}`] : []),
    ...d.tags.map((t) => `ai_guard.attack_category:${t}`),
    ...[...new Set(d.sdsTags)].map((t) => `ai_guard.sds:${t}`),
  ];
  return (
    <div className="aig-side">
      <div className={`aig-verdict${unsafe ? ' unsafe' : ''}`}>
        <b>{unsafe ? 'Unsafe evaluation' : 'Safe evaluation'}</b>
        <span>
          {unsafe ? (blocked ? 'This unsafe request was blocked' : d.blockingEnabled ? 'Flagged; the SDK did not block it' : 'Flagged but not blocked: the service is in monitoring mode') : d.reason}
        </span>
      </div>
      <h5>
        {flagged.length} Attack categor{flagged.length === 1 ? 'y' : 'ies'}
      </h5>
      {d.scores.length ? (
        <ul className="aig-scores">
          {d.scores.map((s) => (
            <li key={s.category} className={s.flagged ? '' : 'below'} title={s.description}>
              <CategoryChip category={s.category} />
              <span className="aig-grow" />
              <Confidence value={s.confidence} flagged={s.flagged} />
              <span className="mono aig-pct">{(s.confidence * 100).toFixed(1)}%</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">No rule matched.</p>
      )}
      <p className="muted aig-threshold">Categories at or above {(d.threshold * 100).toFixed(0)}% confidence are reported.</p>
      {sds.length > 0 && (
        <>
          <h5>Sensitive data</h5>
          <ul className="aig-sds-list">
            {sds.map((f, i) => (
              <li key={i}>
                <span className="aig-sds">{f.rule_tag}</span> {f.rule_display_name} <span className="muted mono">{f.location.path}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      <h5>Tags</h5>
      <div className="aig-tags">
        {(allTags ? tags : tags.slice(0, 2)).map((t) => (
          <span key={t} className="aig-chip">
            {t}
          </span>
        ))}
        {tags.length > 2 && (
          <button className="aig-chip link" onClick={() => setAllTags(!allTags)}>
            {allTags ? 'less' : `+${tags.length - 2}`}
          </button>
        )}
      </div>
    </div>
  );
}

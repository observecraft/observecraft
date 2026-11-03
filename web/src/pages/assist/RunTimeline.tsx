// A run's progress as a timeline: each step in words (what it looked at, over which window, what it found),
// Assist's own notes between steps, times, and a live indicator while it works. Details open on click.
import { useEffect, useState, type ReactNode } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {
  Activity,
  Boxes,
  Bug,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Code2,
  Flag,
  Gauge,
  LineChart,
  LoaderCircle,
  ScrollText,
  ShieldCheck,
  Siren,
  Sparkles,
  Wrench,
  type LucideIcon,
} from 'lucide-react';
import type { RunDetail, RunStep } from '../../api/assist/client';
import { describeStep, stepCounts, type StepKind } from './stepText';

const ICON: Record<StepKind, LucideIcon> = {
  logs: ScrollText,
  metrics: LineChart,
  apm: Activity,
  monitors: Gauge,
  incidents: Siren,
  errors: Bug,
  kubernetes: Boxes,
  security: ShieldCheck,
  code: Code2,
  conclude: Flag,
  other: Wrench,
};

const KIND_LABEL: Record<StepKind, [string, string]> = {
  logs: ['log search', 'log searches'],
  metrics: ['metric query', 'metric queries'],
  apm: ['APM check', 'APM checks'],
  monitors: ['monitor check', 'monitor checks'],
  incidents: ['incident check', 'incident checks'],
  errors: ['Error Tracking search', 'Error Tracking searches'],
  kubernetes: ['Kubernetes check', 'Kubernetes checks'],
  security: ['security check', 'security checks'],
  code: ['code read', 'code reads'],
  conclude: ['conclusion', 'conclusions'],
  other: ['other step', 'other steps'],
};

export function elapsed(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s` : `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

const clock = (at: number) => new Date(at).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });

/** A result line from before results had a readable first line (raw JSON): not worth showing as text. */
const isRaw = (s: string) => /^[[{]/.test(s.trim());

function Note({ text }: { text: string }) {
  if (!text.trim()) return null;
  return (
    <li className="ai-tl-note">
      <span className="ai-tl-dot ai-note">
        <Sparkles size={12} />
      </span>
      <div className="ai-md">
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ img: () => null, a: ({ children }) => <>{children}</> }}>
          {text}
        </ReactMarkdown>
      </div>
    </li>
  );
}

function Tool({ s, live }: { s: Extract<RunStep, { type: 'tool' }>; live: boolean }) {
  const [open, setOpen] = useState(false);
  const d = describeStep(s.name, (s.input ?? {}) as Record<string, unknown>);
  const Icon = ICON[d.kind];
  const running = s.status === 'running' || (live && s.status === 'pending');
  const failed = s.status === 'error';
  let result: ReactNode = null;
  if (running) result = <span className="muted">Running…</span>;
  else if (failed) result = <span className="ai-bad">{s.summary || 'Failed'}</span>;
  else if (s.summary && !isRaw(s.summary)) result = <span>{s.summary}</span>;
  return (
    <li className={`ai-tl-item${running ? ' running' : ''}${failed ? ' failed' : ''}`}>
      <span className={`ai-tl-dot k-${d.kind}`}>{running ? <LoaderCircle size={14} className="ai-spin" /> : failed ? <CircleAlert size={14} /> : <Icon size={14} />}</span>
      <div className="ai-tl-main">
        <button className="ai-tl-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
          <span className="ai-tl-title">
            <b>{d.title}</b>
            {d.what && <code className="ai-tl-what">{d.what}</code>}
            {d.when && <span className="muted"> · {d.when}</span>}
          </span>
          <span className="ai-tl-meta">
            {s.at != null && <span>{clock(s.at)}</span>}
            {s.ms != null && <span>{elapsed(s.ms)}</span>}
            {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </span>
        </button>
        {result && <div className="ai-tl-result">{result}</div>}
        {open && (
          <div className="ai-tl-details">
            <div className="muted">Input</div>
            <pre>{JSON.stringify(s.input, null, 2)}</pre>
            {s.summary && (
              <>
                <div className="muted">Result</div>
                <pre>{s.summary}</pre>
              </>
            )}
          </div>
        )}
      </div>
    </li>
  );
}

/** Ticks every second while the run works. */
function useNow(active: boolean) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [active]);
  return now;
}

export function RunTimeline({ run }: { run: RunDetail }) {
  const live = run.status === 'queued' || run.status === 'running';
  const now = useNow(live);
  const tools = run.steps.filter((s) => s.type === 'tool');
  const counts = stepCounts(run.steps);
  const started = run.startedAt ?? run.createdAt;
  const took = (run.finishedAt ?? now) - started;
  return (
    <section className="ai-card ai-tl-card">
      <div className="ai-tl-header">
        <h2>Timeline</h2>
        <span className="ai-tl-summary">
          {tools.length > 0 && <b>{tools.length} steps</b>}
          {counts.map((c) => {
            const Icon = ICON[c.kind];
            return (
              <span key={c.kind} className={`ai-tl-count k-${c.kind}`} title={`${c.count} ${KIND_LABEL[c.kind][c.count === 1 ? 0 : 1]}`}>
                <Icon size={12} /> {c.count}
              </span>
            );
          })}
          {run.status !== 'queued' && <span className="muted">{live ? `${elapsed(took)} so far` : `took ${elapsed(took)}`}</span>}
        </span>
      </div>
      <ol className="ai-tl">
        <li className="ai-tl-item milestone">
          <span className="ai-tl-dot start">
            <Flag size={12} />
          </span>
          <div className="ai-tl-main">
            <b>Started</b> <span className="muted">by {run.createdBy}</span>
            {run.startedAt && <span className="ai-tl-meta">{clock(run.startedAt)}</span>}
          </div>
        </li>
        {run.steps.map((s, i) => (s.type === 'text' ? <Note key={i} text={s.text} /> : <Tool key={i} s={s} live={live} />))}
        {live && (
          <li className="ai-tl-item running live">
            <span className="ai-tl-dot">
              <LoaderCircle size={14} className="ai-spin" />
            </span>
            <div className="ai-tl-main">
              <b>{run.status === 'queued' ? 'Waiting for a free slot…' : 'Assist is working…'}</b> <span className="muted">{elapsed(now - (run.status === 'queued' ? run.createdAt : started))}</span>
            </div>
          </li>
        )}
        {!live && (
          <li className="ai-tl-item milestone">
            <span className={`ai-tl-dot end ${run.status}`}>
              <Flag size={12} />
            </span>
            <div className="ai-tl-main">
              <b>{run.status === 'concluded' ? 'Concluded' : run.status === 'cancelled' ? 'Cancelled' : 'Failed'}</b>
              {run.finishedAt && <span className="ai-tl-meta">{clock(run.finishedAt)}</span>}
            </div>
          </li>
        )}
      </ol>
    </section>
  );
}

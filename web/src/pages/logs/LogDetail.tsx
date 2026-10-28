import { useEffect, useState, type ReactNode } from 'react';
import { Box, Check, Copy, Hexagon, ListTree, Search, X } from 'lucide-react';
import type { LogEvent } from '../../api/types';
import { formatLongDate, timeAgo } from '../../lib/format';
import { JsonTree } from '../../components/json/JsonTree';
import { LogMessage } from './LogMessage';
import { API_MODE } from '../../api/http';
import { useTrace } from '../apm/queries';

interface Props {
  log: LogEvent;
  onClose: () => void;
  onPrev?: () => void;
  onNext?: () => void;
  /** Adds `key:value` to the current query. */
  onFilter: (key: string, value: string) => void;
  /** The logs around this one from the same host and service. */
  onViewInContext?: () => void;
}

const TAGS_COLLAPSED = 9;
const TABS = ['Event Attributes', 'Trace', 'Metrics', 'Processes'] as const;

function Card({ label, icon, value, onFilter }: { label: string; icon?: ReactNode; value?: string; onFilter: () => void }) {
  if (!value) return null;
  return (
    <button className="detail-card" onClick={onFilter} title={`Filter by ${label.toLowerCase()}:${value}`}>
      <span className="detail-card-label">{label}</span>
      <span className="detail-card-value">
        {icon}
        <span>{value}</span>
      </span>
    </button>
  );
}

/** Right-hand log side panel: status/time header, key infra cards, tags, message, attributes. */
export function LogDetail({ log, onClose, onPrev, onNext, onFilter, onViewInContext }: Props) {
  const [showAllTags, setShowAllTags] = useState(false);
  const [tab, setTab] = useState<(typeof TABS)[number]>('Event Attributes');
  const [attrSearch, setAttrSearch] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === 'INPUT') return;
      if (e.key === 'Escape') onClose();
      if (e.key === 'ArrowUp' && onPrev) (e.preventDefault(), onPrev());
      if (e.key === 'ArrowDown' && onNext) (e.preventDefault(), onNext());
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, onPrev, onNext]);

  const tags = Object.entries(log.tags).sort(([a], [b]) => a.localeCompare(b));
  const visibleTags = showAllTags ? tags : tags.slice(0, TAGS_COLLAPSED);

  const copy = () => {
    navigator.clipboard?.writeText(log.message);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <aside className="log-detail" role="dialog" aria-label="Log details">
      <header className="detail-header">
        <span className={`status-badge status-badge-${log.status}`}>{log.status.toUpperCase()}</span>
        <strong>{formatLongDate(log.timestamp)}</strong>
        <span className="muted-text">({timeAgo(log.timestamp)})</span>
        <span className="detail-header-actions">
          {onViewInContext && (
            <button className="btn" onClick={onViewInContext} title="Logs from the same host and service, 15 minutes around this one">
              <ListTree size={14} /> View in context
            </button>
          )}
          <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
            <X size={20} />
          </button>
        </span>
      </header>

      <div className="detail-cards">
        <Card label="HOST" icon={<Hexagon size={15} />} value={log.host} onFilter={() => onFilter('host', log.host)} />
        <Card label="SERVICE" value={log.service} onFilter={() => onFilter('service', log.service)} />
        <Card label="SOURCE" value={log.source} onFilter={() => onFilter('source', log.source)} />
        <Card label="CONTAINER NAME" icon={<Box size={15} />} value={log.tags.container_name} onFilter={() => onFilter('container_name', log.tags.container_name)} />
        <Card label="DOCKER IMAGE" value={log.tags.docker_image} onFilter={() => onFilter('docker_image', log.tags.docker_image)} />
        <Card label="POD NAME" value={log.tags.pod_name} onFilter={() => onFilter('pod_name', log.tags.pod_name)} />
      </div>

      <section className="detail-section">
        <h4>ALL TAGS</h4>
        <div className="detail-tags">
          {visibleTags.map(([k, v]) => (
            <button key={k} className="detail-tag" title={`Filter by ${k}:${v}`} onClick={() => onFilter(k, v)}>
              {k}:<span>{v}</span>
            </button>
          ))}
          {tags.length > TAGS_COLLAPSED && (
            <button className="detail-tag-more" onClick={() => setShowAllTags((s) => !s)}>
              {showAllTags ? 'Show less' : `+${tags.length - TAGS_COLLAPSED}`}
            </button>
          )}
        </div>
      </section>

      <section className="detail-section">
        <h4>LOG MESSAGE</h4>
        <div className="detail-message mono">
          <div className="detail-message-text">
            <LogMessage message={log.message} />
          </div>
          <span className="detail-message-actions">
            <button title="Copy message" onClick={copy}>
              {copied ? <Check size={15} /> : <Copy size={15} />}
            </button>
          </span>
        </div>
      </section>

      <nav className="detail-tabs">
        {TABS.map((t) => (
          <button key={t} className={t === tab ? 'active' : ''} onClick={() => setTab(t)}>
            {t}
            {t === 'Trace' && <span className="tab-count">{log.traceId ? 1 : 0}</span>}
          </button>
        ))}
      </nav>

      <div className="detail-tab-body">
        {tab === 'Event Attributes' ? (
          <>
            <div className="attr-search">
              {attrSearch !== null && <input autoFocus placeholder="Filter attributes" value={attrSearch} onChange={(e) => setAttrSearch(e.target.value)} />}
              <button className="icon-square" title="Search attributes" onClick={() => setAttrSearch((s) => (s === null ? '' : null))}>
                <Search size={15} />
              </button>
            </div>
            <JsonTree data={log.attributes} filter={attrSearch ?? ''} onFilter={onFilter} />
          </>
        ) : tab === 'Trace' && log.traceId && API_MODE === 'live' ? (
          <LinkedTrace traceId={log.traceId} spanId={log.spanId ?? null} />
        ) : (
          <div className="detail-empty">
            {tab === 'Trace' && 'No trace is linked to this log. Send OTLP logs from an instrumented service, or set trace_id in JSON logs, to correlate them with APM.'}
            {tab === 'Metrics' && `Host metrics for ${log.host} around ${formatLongDate(log.timestamp)} will appear here.`}
            {tab === 'Processes' && `Processes running on ${log.host} will appear here once the process agent reports.`}
          </div>
        )}
      </div>
    </aside>
  );
}

/** The trace a log was written in: its root request, the span that logged, and a link into APM. */
function LinkedTrace({ traceId, spanId }: { traceId: string; spanId: string | null }) {
  const q = useTrace(traceId);
  const href = `/apm/traces?trace=${traceId}${spanId ? `&span=${spanId}` : ''}`;
  if (q.isPending) return <div className="detail-empty">Loading trace…</div>;
  if (q.isError || !q.data.spans.length)
    return (
      <div className="detail-empty">
        Trace <code>{traceId}</code> is no longer stored (traces are kept for a shorter time than logs).
      </div>
    );
  const spans = q.data.spans;
  const root = spans.find((s) => !s.parentSpanId) ?? spans[0];
  const own = spans.find((s) => s.spanId === spanId);
  const errors = spans.filter((s) => s.status === 'Error').length;
  const services = new Set(spans.map((s) => s.service)).size;
  return (
    <div className="linked-trace">
      <div className="linked-trace-row">
        <span className="muted">Request</span>
        <b>
          {root.service} · {root.name}
        </b>
      </div>
      {own && own !== root && (
        <div className="linked-trace-row">
          <span className="muted">Logged by</span>
          <span>
            {own.service} · {own.name} ({(own.durationUs / 1000).toFixed(1)} ms)
          </span>
        </div>
      )}
      <div className="linked-trace-row">
        <span className="muted">Trace</span>
        <span>
          {q.data.durationMs.toFixed(1)} ms · {spans.length} spans · {services} service{services === 1 ? '' : 's'}
          {errors > 0 && (
            <span className="linked-trace-err">
              {' '}
              · {errors} error{errors === 1 ? '' : 's'}
            </span>
          )}
        </span>
      </div>
      <div className="linked-trace-row">
        <span className="muted">Trace ID</span>
        <code>{traceId}</code>
      </div>
      <a className="btn btn-primary btn-sm" href={href}>
        View trace in APM
      </a>
    </div>
  );
}

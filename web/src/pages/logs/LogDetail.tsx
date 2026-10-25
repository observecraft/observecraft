import { useEffect, useState, type ReactNode } from 'react';
import { Box, Check, Copy, Hexagon, ScanSearch, Search, Share, X } from 'lucide-react';
import type { LogEvent } from '../../api/types';
import { formatLongDate, timeAgo } from '../../lib/format';
import { JsonTree } from '../../components/json/JsonTree';
import { LogMessage } from './LogMessage';

interface Props {
  log: LogEvent;
  onClose: () => void;
  onPrev?: () => void;
  onNext?: () => void;
  /** Adds `key:value` to the current query. */
  onFilter: (key: string, value: string) => void;
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
export function LogDetail({ log, onClose, onPrev, onNext, onFilter }: Props) {
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
          <button className="icon-square" title="Share">
            <Share size={16} />
          </button>
          <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
            <X size={20} />
          </button>
        </span>
      </header>

      <div className="detail-cards">
        <Card label="HOST" icon={<Hexagon size={15} />} value={log.host} onFilter={() => onFilter('host', log.host)} />
        <Card label="SERVICE" value={log.service} onFilter={() => onFilter('service', log.service)} />
        <Card label="SOURCE" value={log.source} onFilter={() => onFilter('source', log.source)} />
        <Card
          label="CONTAINER NAME"
          icon={<Box size={15} />}
          value={log.tags.container_name}
          onFilter={() => onFilter('container_name', log.tags.container_name)}
        />
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
            <button title="Add finding">
              <ScanSearch size={15} />
            </button>
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
            {t === 'Trace' && <span className="tab-count">0</span>}
          </button>
        ))}
      </nav>

      <div className="detail-tab-body">
        {tab === 'Event Attributes' ? (
          <>
            <div className="attr-search">
              {attrSearch !== null && (
                <input
                  autoFocus
                  placeholder="Filter attributes"
                  value={attrSearch}
                  onChange={(e) => setAttrSearch(e.target.value)}
                />
              )}
              <button
                className="icon-square"
                title="Search attributes"
                onClick={() => setAttrSearch((s) => (s === null ? '' : null))}
              >
                <Search size={15} />
              </button>
            </div>
            <JsonTree data={log.attributes} filter={attrSearch ?? ''} onFilter={onFilter} />
          </>
        ) : (
          <div className="detail-empty">
            {tab === 'Trace' && 'No trace is linked to this log. Inject trace_id into your logs to correlate them with APM.'}
            {tab === 'Metrics' && `Host metrics for ${log.host} around ${formatLongDate(log.timestamp)} will appear here.`}
            {tab === 'Processes' && `Processes running on ${log.host} will appear here once the process agent reports.`}
          </div>
        )}
      </div>
    </aside>
  );
}

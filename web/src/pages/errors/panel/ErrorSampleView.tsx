import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { AlertCircle, ArrowDownWideNarrow, ChevronDown, ChevronLeft, ChevronRight, FileText, Fingerprint, MousePointerClick, Network, RotateCw, Search, Star, XCircle } from 'lucide-react';
import type { ErrorSample, IssueDetail, TimelineEvent } from '../../../api/errorTracking/types';
import type { TimeRange } from '../../../api/types';
import { JsonTree } from '../../../components/json/JsonTree';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { formatLogDate, timeAgo } from '../../../lib/format';
import { useErrorSample } from '../queries';
import { StackTrace } from './StackTrace';

const SECTIONS = ['Stack Trace', 'Events Timeline', 'Event Attributes', 'JSON'] as const;
type Section = (typeof SECTIONS)[number];

type Order = 'recommended' | 'latest' | 'oldest';

interface Props {
  issue: IssueDetail;
  range: TimeRange;
  query: string;
}

export function ErrorSampleView({ issue, range, query }: Props) {
  const navigate = useNavigate();
  const [order, setOrder] = useState<Order>('recommended');
  const [index, setIndex] = useState(0);
  const sampleQ = useErrorSample({ issueId: issue.id, range, query, index, order: order === 'oldest' ? 'oldest' : 'latest' });

  useEffect(() => setIndex(0), [issue.id, query, order]);

  const total = sampleQ.data?.total ?? 0;
  const available = sampleQ.data?.available ?? 0;
  const sample = sampleQ.data?.sample;

  return (
    <section className="es">
      <header className="es-header">
        <h3>Error sample</h3>
        <Select
          value={order}
          onChange={setOrder}
          options={[
            { value: 'recommended', label: 'Recommended', hint: 'most recent with full context' },
            { value: 'latest', label: 'Latest' },
            { value: 'oldest', label: 'Oldest' },
          ]}
          triggerClassName="link-btn es-order"
          trigger={(o) => (
            <>
              <Star size={15} /> {o?.label} <ChevronDown size={14} />
            </>
          )}
        />
        <span className="es-count muted-text">{total > 0 && `${index + 1} of ${total.toLocaleString()}`}</span>
        <div className="es-nav">
          <div className="btn-group">
            <button className="btn" disabled={index === 0} onClick={() => setIndex((i) => i - 1)}>
              <ChevronLeft size={14} /> Previous
            </button>
            <button className="btn" disabled={index >= available - 1} onClick={() => setIndex((i) => i + 1)}>
              Next <ChevronRight size={14} />
            </button>
          </div>
          <button className="btn btn-primary" onClick={() => navigate(`/logs?query=${encodeURIComponent(`service:${issue.service} status:error`)}`)}>
            View All Errors
          </button>
        </div>
      </header>

      {sampleQ.isPending ? (
        <Loading label="Loading sample…" />
      ) : sampleQ.isError ? (
        <ErrorState error={sampleQ.error} onRetry={() => sampleQ.refetch()} />
      ) : !sample ? (
        <div className="es-card state-box">No error samples match this filter in the selected time range.</div>
      ) : (
        <SampleCard sample={sample} stale={sampleQ.isPlaceholderData} />
      )}
    </section>
  );
}

function SampleCard({ sample, stale }: { sample: ErrorSample; stale: boolean }) {
  const [active, setActive] = useState<Section>('Stack Trace');
  const refs = useRef<Partial<Record<Section, HTMLElement | null>>>({});
  const [attrSearch, setAttrSearch] = useState<string | null>(null);

  const jump = (s: Section) => {
    setActive(s);
    refs.current[s]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  const shownTags = Object.entries(sample.tags).filter(([k]) => k === 'env');

  return (
    <div className={`es-card${stale ? ' is-stale' : ''}`}>
      <div className="es-meta">
        <div>
          <strong>{formatLogDate(sample.timestamp)}</strong> <span className="muted-text">({timeAgo(sample.timestamp)})</span>
        </div>
        <div className="es-tags">
          {shownTags.map(([k, v]) => (
            <span key={k} className="tag-chip">
              {k}:{v}
            </span>
          ))}
        </div>
      </div>

      <nav className="es-pills">
        {SECTIONS.map((s) => (
          <button key={s} className={s === active ? 'active' : ''} onClick={() => jump(s)}>
            {s}
          </button>
        ))}
      </nav>

      <Collapsible title="Stack Trace" sectionRef={(el) => (refs.current['Stack Trace'] = el)}>
        <StackTrace stack={sample.stack} />
      </Collapsible>

      <Collapsible
        title="Events Timeline"
        subtitle="View the Log events and RUM events before and after this error occurred"
        extra={
          <span className="session-badge">
            <Fingerprint size={13} /> {sample.timeline.length} Session Events
          </span>
        }
        sectionRef={(el) => (refs.current['Events Timeline'] = el)}
      >
        <Timeline events={sample.timeline} />
      </Collapsible>

      <Collapsible title="Event Attributes" sectionRef={(el) => (refs.current['Event Attributes'] = el)}>
        <div className="attr-groups">
          {sample.attributeGroups.map((g) => (
            <div key={g.name} className="attr-group">
              <div className="attr-group-name">{g.name}</div>
              <div className="attr-chips">
                {g.attributes.map((a) => (
                  <span key={a.label} className="attr-chip">
                    {a.label}:<span>{a.value}</span>
                  </span>
                ))}
              </div>
            </div>
          ))}
        </div>
      </Collapsible>

      <Collapsible title="JSON" sectionRef={(el) => (refs.current.JSON = el)}>
        <div className="es-json">
          <div className="attr-search">
            {attrSearch !== null && <input autoFocus placeholder="Filter keys and values" value={attrSearch} onChange={(e) => setAttrSearch(e.target.value)} />}
            <button className="icon-square" title="Search JSON" onClick={() => setAttrSearch((s) => (s === null ? '' : null))}>
              <Search size={15} />
            </button>
          </div>
          <JsonTree data={sample.json} filter={attrSearch ?? ''} expandDepth={3} />
        </div>
      </Collapsible>
    </div>
  );
}

function Collapsible({ title, subtitle, extra, sectionRef, children }: { title: string; subtitle?: string; extra?: ReactNode; sectionRef: (el: HTMLElement | null) => void; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="es-section" ref={sectionRef}>
      <button className="es-section-head" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
        <strong>{title}</strong>
        {subtitle && <span className="muted-text">{subtitle}</span>}
        {extra && <span className="es-section-extra">{extra}</span>}
      </button>
      {open && <div className="es-section-body">{children}</div>}
    </section>
  );
}

const TYPE_ICON: Record<TimelineEvent['type'], ReactNode> = {
  crash: <AlertCircle size={14} className="tl-crash" fill="currentColor" stroke="var(--bg)" />,
  error: <XCircle size={14} className="tl-crash" />,
  view: <RotateCw size={14} className="tl-view" />,
  action: <MousePointerClick size={14} className="tl-view" />,
  resource: <Network size={14} className="tl-view" />,
  log: <FileText size={14} className="tl-log" />,
};

function Timeline({ events }: { events: TimelineEvent[] }) {
  return (
    <table className="timeline">
      <thead>
        <tr>
          <th className="tl-gutter" />
          <th>
            <ArrowDownWideNarrow size={13} /> TIME
          </th>
          <th>TYPE</th>
          <th>MESSAGE</th>
        </tr>
      </thead>
      <tbody>
        {events.map((e) => (
          <tr key={`${e.at}-${e.label}`} className={e.isCurrent ? 'current' : ''}>
            <td className="tl-gutter">
              <FileText size={14} className="muted" />
            </td>
            <td>{formatLogDate(e.at)}</td>
            <td>
              <span className="tl-type">
                {TYPE_ICON[e.type]} {e.label}
              </span>
            </td>
            <td>
              {e.message} {e.emphasis && <strong>{e.emphasis}</strong>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUp, ChevronDown, Pause, Play, Settings, Trash2, Zap } from 'lucide-react';
import { logHistogram, subscribeLiveTail, suggestLogQuery } from '../../api/client';
import type { HistogramBucket, LogEvent, TimeRange } from '../../api/types';
import { utcOffset } from '../../components/shell/PageHeader';
import { addTerm } from '../../lib/searchQuery';
import { AggregateView, type VizMode } from './AggregateView';
import { LogDetail } from './LogDetail';
import { LogHistogram } from './LogHistogram';
import { DEFAULT_TABLE_OPTIONS, LogTable, OptionsMenu, type LogTableOptions } from './LogTable';
import { QueryBar } from '../../components/query/QueryBar';
import { VizPicker } from './VizPicker';

const WINDOW_MS = 15 * 60e3;
const MAX_BUFFER = 1000;
const HISTOGRAM_REFRESH_MS = 5000;
const LIVE_MODES: VizMode[] = ['list', 'timeseries', 'toplist', 'bar', 'table', 'treemap', 'pie'];

function lastWindow(): TimeRange {
  const to = Date.now();
  return { from: to - WINDOW_MS, to };
}

export function LiveTail() {
  const [query, setQuery] = useState('');
  const [viz, setViz] = useState<VizMode>('list');
  const [paused, setPaused] = useState(false);
  const [buffer, setBuffer] = useState<LogEvent[]>([]);
  const [pending, setPending] = useState<LogEvent[]>([]);
  const [stats, setStats] = useState({ rate: 0, ratio: 1 });
  const [range, setRange] = useState<TimeRange>(lastWindow);
  const [buckets, setBuckets] = useState<HistogramBucket[]>([]);
  const [selected, setSelected] = useState<LogEvent | null>(null);
  const [options, setOptions] = useState<LogTableOptions>(DEFAULT_TABLE_OPTIONS);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  // A new query starts a fresh tail.
  useEffect(() => {
    setBuffer([]);
    setPending([]);
  }, [query]);

  useEffect(() => {
    if (paused) return;
    return subscribeLiveTail(query, (batch) => {
      setStats({ rate: batch.matchedPerSec, ratio: batch.displayedRatio });
      if (batch.events.length === 0) return;
      const newest = [...batch.events].reverse();
      // While the user is scrolled down reading, hold new rows instead of shifting the list.
      if ((scroller.current?.scrollTop ?? 0) > 40) setPending((p) => [...newest, ...p].slice(0, MAX_BUFFER));
      else setBuffer((b) => [...newest, ...b].slice(0, MAX_BUFFER));
    });
  }, [query, paused]);

  useEffect(() => {
    if (paused) return;
    let cancelled = false;
    const refresh = () => {
      const r = lastWindow();
      setRange(r);
      logHistogram({ range: r, query }).then((b) => !cancelled && setBuckets(b));
    };
    refresh();
    const id = setInterval(refresh, HISTOGRAM_REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [query, paused]);

  const flushPending = useCallback(() => {
    setBuffer((b) => [...pending, ...b].slice(0, MAX_BUFFER));
    setPending([]);
  }, [pending]);

  const onScroll = () => {
    if (pending.length && (scroller.current?.scrollTop ?? 0) <= 40) flushPending();
  };

  const suggest = useCallback((partial: string) => suggestLogQuery(range, partial), [range]);
  const addFilter = useCallback((key: string, value: string) => setQuery((q) => addTerm(q, key, value)), []);

  const selIdx = selected ? buffer.findIndex((l) => l.id === selected.id) : -1;

  return (
    <div className="live-tail">
      <div className="section-header">
        <div className="section-title">
          <span className="live-icon">
            <Zap size={15} fill="currentColor" />
          </span>
          Live Tail
          <span className="section-subtitle">All Ingested Data</span>
        </div>
        <div className="time-controls">
          <div className="time-picker">
            <span className="tz">{utcOffset()}</span>
            <div className="time-picker-btn live-range">
              <span className={`live-badge${paused ? ' paused' : ''}`}>
                <Zap size={11} fill="currentColor" /> {paused ? 'PAUSED' : 'LIVE'}
              </span>
              <span className="time-label">Past 15 Minutes</span>
              <ChevronDown size={14} className="muted" />
            </div>
          </div>
          <button className="btn" onClick={() => setPaused((p) => !p)}>
            {paused ? <Play size={14} fill="currentColor" /> : <Pause size={14} fill="currentColor" />}
            {paused ? 'Play' : 'Pause'}
          </button>
        </div>
      </div>

      <div className="log-query">
        <QueryBar value={query} onChange={setQuery} suggest={suggest} placeholder="Filter the live stream, e.g. service:storefront-api env:prod" />
      </div>

      <VizPicker value={viz} onChange={setViz} modes={LIVE_MODES} />

      <div className="log-histogram live-histogram">
        <LogHistogram buckets={buckets} height={92} />
      </div>

      <div className="live-stats">
        <strong>{paused ? `Paused, ${buffer.length.toLocaleString()} events displayed` : `${Math.round(stats.rate)} events/s, ${(stats.ratio * 100).toFixed(2)}% displayed`}</strong>
        <div className="live-actions">
          <button className="btn" onClick={() => (setBuffer([]), setPending([]))}>
            <Trash2 size={14} /> Clear
          </button>
          <div className="options-anchor">
            <button className="btn" onClick={() => setOptionsOpen((o) => !o)}>
              <Settings size={14} /> Options
            </button>
            {optionsOpen && <OptionsMenu value={options} onChange={setOptions} />}
          </div>
        </div>
      </div>

      <div className="live-list" ref={scroller} onScroll={onScroll}>
        {pending.length > 0 && (
          <button
            className="pending-banner"
            onClick={() => {
              scroller.current?.scrollTo({ top: 0 });
              flushPending();
            }}
          >
            <ArrowUp size={14} /> {pending.length} new logs
          </button>
        )}
        {viz === 'list' ? (
          <LogTable logs={buffer} selectedId={selected?.id ?? null} onSelect={setSelected} options={options} flashNew />
        ) : (
          <div className="agg-wrap">
            <AggregateView mode={viz as Exclude<VizMode, 'list'>} logs={buffer} histogram={buckets} />
          </div>
        )}
      </div>

      {selected && (
        <LogDetail
          key={selected.id}
          log={selected}
          onClose={() => setSelected(null)}
          onPrev={selIdx > 0 ? () => setSelected(buffer[selIdx - 1]) : undefined}
          onNext={selIdx >= 0 && selIdx < buffer.length - 1 ? () => setSelected(buffer[selIdx + 1]) : undefined}
          onFilter={addFilter}
        />
      )}
    </div>
  );
}

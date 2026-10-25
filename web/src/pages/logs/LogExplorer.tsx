import { useCallback, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import {
  ChevronDown,
  Download,
  EllipsisVertical,
  LayoutDashboard,
  PanelLeft,
  ScanSearch,
  Settings,
  Sheet,
} from 'lucide-react';
import { searchLogs, suggestLogQuery } from '../../api/client';
import type { LogEvent, LogInsight, LogSearchResult } from '../../api/types';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { formatLogDate } from '../../lib/format';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { AggregateView, type VizMode } from './AggregateView';
import { InsightsPanel } from './InsightsPanel';
import { LogDetail } from './LogDetail';
import { LogHistogram } from './LogHistogram';
import { LogTable } from './LogTable';
import { VizPicker } from './VizPicker';

const MAX_ROWS = 500;

function downloadCsv(logs: LogEvent[]) {
  const esc = (s: string) => `"${s.replace(/"/g, '""')}"`;
  const rows = [['date', 'status', 'host', 'service', 'message']].concat(
    logs.map((l) => [formatLogDate(l.timestamp), l.status, l.host, l.service, l.message]),
  );
  const blob = new Blob([rows.map((r) => r.map(esc).join(',')).join('\n')], { type: 'text/csv' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = 'logs.csv';
  a.click();
  URL.revokeObjectURL(a.href);
}

export function LogExplorer() {
  const { range } = useTime();
  // Deep links (e.g. Error Tracking → "View All Errors") seed the query via ?query=…
  const [params] = useSearchParams();
  const [query, setQuery] = useState(() => params.get('query') ?? '');
  const [viz, setViz] = useState<VizMode>('list');
  const [showFacets, setShowFacets] = useState(true);
  const [selectedLog, setSelectedLog] = useState<LogEvent | null>(null);
  const [result, setResult] = useState<LogSearchResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    searchLogs({ range, query }).then((r) => {
      if (!cancelled) setResult(r);
    });
    return () => {
      cancelled = true;
    };
  }, [range, query]);

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((next: Term[]) => setQuery(formatQuery(next)), []);
  const suggest = useCallback((partial: string) => suggestLogQuery(range, partial), [range]);
  const addFilter = useCallback((key: string, value: string) => setQuery((q) => addTerm(q, key, value)), []);
  const pickInsight = useCallback((i: LogInsight) => addFilter(i.attribute, i.value), [addFilter]);

  const logs = result?.logs ?? [];
  const rows = logs.slice(0, MAX_ROWS);
  // The panel keeps its log even if a refresh pushes it out of the result set.
  const selIdx = selectedLog ? rows.findIndex((l) => l.id === selectedLog.id) : -1;

  return (
    <div className="log-explorer">
      <div className="section-header">
        <div className="section-header-left">
          <div className="page-tab">
            <ScanSearch size={18} /> Findings <span className="badge-new">NEW</span>
          </div>
          <div className="page-tab active">
            My View
            <EllipsisVertical size={16} className="muted" />
          </div>
        </div>
        <TimePicker />
      </div>

      <div className="log-scroll">
        <div className="log-query">
          <QueryBar value={query} onChange={setQuery} suggest={suggest} placeholder="Search for logs, e.g. service:order-service status:error" />
        </div>

        <VizPicker value={viz} onChange={setViz} />

        <div className="log-histogram">
          <LogHistogram buckets={result?.histogram ?? []} />
        </div>

        <div className="log-body">
          {showFacets && result && <FacetPanel facets={result.facets} terms={terms} onChange={setTerms} />}

          <div className="log-results">
            <InsightsPanel insights={result?.insights ?? []} onPick={pickInsight} />

            <div className="log-list-card">
              <div className="log-toolbar">
                <button className="toolbar-toggle" onClick={() => setShowFacets((s) => !s)}>
                  <PanelLeft size={14} /> Facets
                </button>
                <span className="log-count">{(result?.total ?? 0).toLocaleString()} logs</span>
                <div className="toolbar-right">
                  <div className="btn-group">
                    <span className="btn btn-label">Open in</span>
                    <button className="btn">
                      <LayoutDashboard size={14} /> Dashboards
                    </button>
                    <button className="btn">
                      <Sheet size={14} /> Sheets
                    </button>
                    <button className="btn">
                      <ChevronDown size={14} />
                    </button>
                  </div>
                  <div className="btn-group">
                    <button className="btn" onClick={() => downloadCsv(logs)}>
                      <Download size={14} /> CSV
                    </button>
                    <button className="btn">
                      <ChevronDown size={14} />
                    </button>
                  </div>
                  <button className="toolbar-link">
                    <ScanSearch size={14} /> Add Finding <span className="badge-new">NEW</span>
                  </button>
                  <button className="toolbar-icon" title="Options">
                    <Settings size={15} />
                  </button>
                </div>
              </div>

              {viz === 'list' ? (
                <LogTable logs={rows} selectedId={selectedLog?.id ?? null} onSelect={setSelectedLog} />
              ) : (
                <div className="agg-wrap">
                  <AggregateView mode={viz} logs={logs} histogram={result?.histogram ?? []} />
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      {selectedLog && (
        <LogDetail
          key={selectedLog.id}
          log={selectedLog}
          onClose={() => setSelectedLog(null)}
          onPrev={selIdx > 0 ? () => setSelectedLog(rows[selIdx - 1]) : undefined}
          onNext={selIdx >= 0 && selIdx < rows.length - 1 ? () => setSelectedLog(rows[selIdx + 1]) : undefined}
          onFilter={addFilter}
        />
      )}
    </div>
  );
}

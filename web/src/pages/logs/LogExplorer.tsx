import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Download, LayoutDashboard, PanelLeft, Settings } from 'lucide-react';
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
import { DEFAULT_TABLE_OPTIONS, LogTable, OptionsMenu, type LogTableOptions } from './LogTable';
import { logStreamWidgets } from '../dashboards/tree';
import { useCreateDashboard } from '../dashboards/queries';
import { updateDashboard } from '../../api/dashboards/client';
import { VizPicker } from './VizPicker';
import { PatternsTable } from './PatternsTable';
import { patternQuery } from './patterns';

const MAX_ROWS = 500;

function downloadCsv(logs: LogEvent[]) {
  const esc = (s: string) => `"${s.replace(/"/g, '""')}"`;
  const rows = [['date', 'status', 'host', 'service', 'message']].concat(logs.map((l) => [formatLogDate(l.timestamp), l.status, l.host, l.service, l.message]));
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
  const [groupInto, setGroupInto] = useState<'fields' | 'patterns'>('fields');
  const [tableOptions, setTableOptions] = useState<LogTableOptions>(DEFAULT_TABLE_OPTIONS);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const navigate = useNavigate();
  const createDashboard = useCreateDashboard();
  // Open in Dashboards: a new dashboard with this search as a log stream.
  const openInDashboard = async () => {
    const d = await createDashboard.mutateAsync({ title: `Logs: ${query.trim() || 'all logs'}`, teams: [], layoutType: 'grid' });
    await updateDashboard(d.id, { widgets: logStreamWidgets(query) });
    navigate(`/dashboards/${d.id}`);
  };
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
          <div className="page-tab active">My View</div>
        </div>
        <TimePicker />
      </div>

      <div className="log-scroll">
        <div className="log-query">
          <QueryBar value={query} onChange={setQuery} suggest={suggest} placeholder="Search for logs, e.g. service:order-service status:error" />
        </div>

        <div className="viz-row">
          <div className="btn-group">
            <span className="btn btn-label">Group into</span>
            {(['fields', 'patterns'] as const).map((g) => (
              <button key={g} className={`btn${groupInto === g ? ' btn-active' : ''}`} onClick={() => setGroupInto(g)}>
                {g === 'fields' ? 'Fields' : 'Patterns'}
              </button>
            ))}
          </div>
        </div>
        {groupInto === 'fields' && <VizPicker value={viz} onChange={setViz} />}

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
                  <button className="btn" onClick={openInDashboard} disabled={createDashboard.isPending}>
                    <LayoutDashboard size={14} /> Open in Dashboards
                  </button>
                  <button className="btn" onClick={() => downloadCsv(logs)}>
                    <Download size={14} /> CSV
                  </button>
                  <div className="options-anchor">
                    <button className={`toolbar-icon${optionsOpen ? ' active' : ''}`} title="Options" onClick={() => setOptionsOpen((o) => !o)}>
                      <Settings size={15} />
                    </button>
                    {optionsOpen && <OptionsMenu value={tableOptions} onChange={setTableOptions} />}
                  </div>
                </div>
              </div>

              {groupInto === 'patterns' ? (
                <PatternsTable
                  range={range}
                  query={query}
                  onOpen={(p) => {
                    setQuery(patternQuery(p, query));
                    setGroupInto('fields');
                    setViz('list');
                  }}
                />
              ) : viz === 'list' ? (
                <LogTable logs={rows} selectedId={selectedLog?.id ?? null} onSelect={setSelectedLog} options={tableOptions} />
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

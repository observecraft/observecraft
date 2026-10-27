import { useCallback, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Download, LayoutDashboard, PanelLeft, Settings } from 'lucide-react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { logFacets, logFields, logHistogram, logInsights, searchLogPage, suggestLogQuery } from '../../api/client';
import type { LogEvent, LogInsight } from '../../api/types';
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

/** localStorage key of the facets added with "+ Add". */
const ADDED_FACETS_KEY = 'oc.logs.addedFacets';

export function LogExplorer() {
  const { range, focus } = useTime();
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

  // Facets this viewer added with "+ Add" (kept in this browser).
  const [addedFacets, setAddedFacets] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(ADDED_FACETS_KEY) ?? '[]') as string[];
    } catch {
      return [];
    }
  });
  const addFacet = useCallback((key: string) => {
    setAddedFacets((list) => {
      const next = list.includes(key) ? list : [...list, key];
      try {
        localStorage.setItem(ADDED_FACETS_KEY, JSON.stringify(next));
      } catch {
        // Private windows: the facet still shows until the page is left.
      }
      return next;
    });
  }, []);

  // Independent requests: the rows show as soon as the search returns, facets and insights when ready.
  const page = useQuery({ queryKey: ['logs', 'page', range, query], queryFn: () => searchLogPage({ range, query }), placeholderData: keepPreviousData });
  const histogram = useQuery({ queryKey: ['logs', 'histogram', range, query], queryFn: () => logHistogram({ range, query }), placeholderData: keepPreviousData });
  const facets = useQuery({ queryKey: ['logs', 'facets', range, query, addedFacets], queryFn: () => logFacets({ range, query, facets: addedFacets }), placeholderData: keepPreviousData });
  const insights = useQuery({ queryKey: ['logs', 'insights', range, query], queryFn: () => logInsights({ range, query }), placeholderData: keepPreviousData });

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((next: Term[]) => setQuery(formatQuery(next)), []);
  const suggest = useCallback((partial: string) => suggestLogQuery(range, partial), [range]);
  const addFilter = useCallback((key: string, value: string) => setQuery((q) => addTerm(q, key, value)), []);
  const pickInsight = useCallback((i: LogInsight) => addFilter(i.attribute, i.value), [addFilter]);

  const logs = page.data?.logs ?? [];
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
          <LogHistogram buckets={histogram.data ?? []} />
        </div>

        <div className="log-body">
          {showFacets && facets.data && <FacetPanel facets={facets.data} terms={terms} onChange={setTerms} addable={{ load: () => logFields({ range, query }), onAdd: addFacet }} />}

          <div className="log-results">
            <InsightsPanel insights={insights.data ?? []} onPick={pickInsight} />

            <div className="log-list-card">
              <div className="log-toolbar">
                <button className="toolbar-toggle" onClick={() => setShowFacets((s) => !s)}>
                  <PanelLeft size={14} /> Facets
                </button>
                <span className="log-count">{(page.data?.total ?? 0).toLocaleString()} logs</span>
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
                  <AggregateView mode={viz} logs={logs} histogram={histogram.data ?? []} />
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
          onViewInContext={() => {
            focus(selectedLog.timestamp);
            setQuery([selectedLog.host && `host:${selectedLog.host}`, selectedLog.service && `service:${selectedLog.service}`].filter(Boolean).join(' '));
          }}
        />
      )}
    </div>
  );
}

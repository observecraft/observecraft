import { useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Search, X } from 'lucide-react';
import type { LlmTraceRow } from '../../api/llm/client';
import { QueryBar } from '../../components/query/QueryBar';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Pager } from '../../components/ui/Pager';
import { Facet, ValuesFacet } from '../monitors/common';
import { fmtLatency } from '../apm/visuals';
import { fmtStamp, KindBadge, StatusDot } from './common';
import { fmtCost, fmtTokens, kindLabel } from './format';
import { useLlmApp, useLlmQuery } from './LlmLayout';
import { LlmTraceView } from './LlmTraceView';
import { useLlmFacets, useLlmTraces } from './queries';

const PAGE = 50;

const FACETS: { id: string; label: string }[] = [
  { id: 'status', label: 'Status' },
  { id: 'ml_app', label: 'ML Application' },
  { id: 'service', label: 'Service' },
  { id: 'model', label: 'Model' },
  { id: 'provider', label: 'Provider' },
  { id: 'kind', label: 'Span Kind' },
  { id: 'operation', label: 'Operation' },
  { id: 'env', label: 'Env' },
];

/** Facet selections from the URL (`?status=error&kind=agent`, as the Views menu links them). */
function initialFilters(params: URLSearchParams): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const f of FACETS) {
    const v = params.get(f.id);
    if (v && f.id !== 'ml_app') out[f.id] = v.split(',');
  }
  return out;
}

/** Investigate → Traces: one row per trace with LLM spans, facets, search and the trace panel. */
export function LlmTracesPage() {
  const [params, setParams] = useSearchParams();
  const [app, setApp] = useLlmApp();
  const [query, setQuery] = useState(params.get('query') ?? '');
  const [filters, setFilters] = useState<Record<string, string[]>>(() => initialFilters(params));
  const [offset, setOffset] = useState(0);
  const [facetSearch, setFacetSearch] = useState('');
  const q = useLlmQuery(query, filters);
  const list = useLlmTraces(q, offset, PAGE);
  // The app facet reflects the header's app selector, so its counts ignore that selection.
  const facets = useLlmFacets(
    q,
    FACETS.map((f) => f.id),
  );
  useEffect(() => setOffset(0), [query, filters, app]);
  const openTrace = params.get('trace');
  const openSpan = params.get('span');
  const setParam = (pairs: Record<string, string | null>) => {
    const n = new URLSearchParams(params);
    for (const [k, v] of Object.entries(pairs)) {
      if (v) n.set(k, v);
      else n.delete(k);
    }
    setParams(n, { replace: true });
  };
  const fs = facetSearch.trim().toLowerCase();
  return (
    <div className="llm-explorer">
      <div className="llm-query">
        <div className="sx-input">
          <QueryBar value={query} onChange={setQuery} placeholder="Search LLM spans: service:support-assistant @gen_ai.request.model:gpt-4o* -status:ok" />
        </div>
        {query && (
          <button className="icon-plain" title="Clear" onClick={() => setQuery('')}>
            <X size={18} />
          </button>
        )}
      </div>
      <div className="llm-explorer-body">
        <aside className="sx-facets llm-facets">
          <label className="mon-facet-search sx-facet-search">
            <Search size={14} />
            <input placeholder="Search facets" value={facetSearch} onChange={(e) => setFacetSearch(e.target.value)} />
          </label>
          {FACETS.filter((f) => !fs || f.label.toLowerCase().includes(fs)).map((f) => (
            <Facet key={f.id} title={f.label} defaultOpen={['status', 'ml_app', 'model', 'kind'].includes(f.id)}>
              <ValuesFacet
                values={facets.data?.facets[f.id] ?? []}
                selected={f.id === 'ml_app' ? (app ? [app] : []) : (filters[f.id] ?? [])}
                onChange={(v) => (f.id === 'ml_app' ? setApp(v[v.length - 1] ?? '') : setFilters((x) => ({ ...x, [f.id]: v })))}
                searchable={['service', 'model', 'ml_app'].includes(f.id)}
                label={f.id === 'status' ? (v) => (v === 'error' ? 'Error' : 'OK') : f.id === 'kind' ? kindLabel : undefined}
                icon={f.id === 'status' ? (v) => <i className={`sx-dot ${v}`} /> : undefined}
              />
            </Facet>
          ))}
        </aside>
        <main className="llm-results">
          <div className="sx-results-head">
            <b>{list.data ? `${list.data.total.toLocaleString()} traces` : '…'}</b>
            <span className="muted">Each row is a trace, shown by its top LLM span.</span>
          </div>
          {list.isPending ? (
            <Loading label="Searching traces…" />
          ) : list.isError ? (
            <ErrorState error={list.error} onRetry={() => list.refetch()} />
          ) : list.data.traces.length === 0 ? (
            <p className="mon-empty">No LLM traces match this search in the selected time range.</p>
          ) : (
            <>
              <TraceTable rows={list.data.traces} selected={openTrace} onOpen={(r) => setParam({ trace: r.traceId, span: r.spanId })} />
              <div className="mon-pager">
                <Pager offset={offset} limit={PAGE} total={Math.min(list.data.total, 10_000 + PAGE)} onChange={setOffset} />
              </div>
            </>
          )}
        </main>
      </div>
      {openTrace && <LlmTraceView traceId={openTrace} focusSpanId={openSpan} onClose={() => setParam({ trace: null, span: null })} onSelectSpan={(id) => setParam({ span: id })} />}
    </div>
  );
}

function TraceTable({ rows, selected, onOpen }: { rows: LlmTraceRow[]; selected: string | null; onOpen: (r: LlmTraceRow) => void }) {
  return (
    <table className="sx-table llm-table llm-trace-table">
      <thead>
        <tr>
          <th className="sx-bar-col" />
          <th>DATE</th>
          <th>ML APP / SERVICE</th>
          <th>SPAN</th>
          <th>KIND</th>
          <th>MODEL</th>
          <th>INPUT</th>
          <th>OUTPUT</th>
          <th className="num">TOKENS</th>
          <th className="num">EST. COST</th>
          <th className="num">DURATION</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.traceId} className={r.traceId === selected ? 'selected' : ''} onClick={() => onOpen(r)}>
            <td className="sx-bar-col">
              <StatusDot status={r.status} />
            </td>
            <td className="sx-date">{fmtStamp(r.timestamp)}</td>
            <td>
              <b>{r.mlApp}</b>
              {r.service !== r.mlApp && <div className="muted llm-sub">{r.service}</div>}
            </td>
            <td className="llm-name" title={r.name}>
              {r.name}
              {r.llmSpans > 1 && <span className="muted llm-sub"> · {r.llmSpans} spans</span>}
            </td>
            <td>
              <KindBadge kind={r.kind} />
            </td>
            <td>{r.model}</td>
            <td className="llm-preview" title={r.inputPreview}>
              {r.inputPreview || <span className="muted">—</span>}
            </td>
            <td className="llm-preview" title={r.outputPreview}>
              {r.outputPreview || <span className="muted">{r.status === 'error' ? 'no output (error)' : '—'}</span>}
            </td>
            <td className="num" title={`${r.inputTokens.toLocaleString()} in · ${r.outputTokens.toLocaleString()} out`}>
              {fmtTokens(r.inputTokens + r.outputTokens)}
            </td>
            <td className="num">{fmtCost(r.estimatedCostUsd)}</td>
            <td className="num">{fmtLatency(r.durationMs)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

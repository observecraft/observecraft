import { Fragment, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ChevronDown, ChevronRight, Cylinder, Search, Server, Timer } from 'lucide-react';
import type { StreamEdge, StreamPathway } from '../../api/dataStreams/client';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { Empty } from '../../components/ui/AsyncState';
import { fmtCount, fmtLagSeconds, fmtMs, fmtRate, isTopic, nodeName, systemLogo } from './format';
import { useStreamMap } from './queries';

function TopicIcon({ id }: { id: string }) {
  const logo = systemLogo(id.split(':')[1]);
  return logo ? <BrandLogo name={logo} size={12} /> : <Cylinder size={12} />;
}

/** service → topic → service … as chips. */
export function PathwayChain({ nodes, highlight }: { nodes: string[]; highlight?: string }) {
  return (
    <span className="dsm-chain">
      {nodes.map((id, i) => (
        <Fragment key={`${id}-${i}`}>
          {i > 0 && <span className="dsm-chain-arrow">→</span>}
          <span className={`dsm-chip${isTopic(id) ? ' topic' : ''}${id === highlight ? ' hl' : ''}`}>
            {isTopic(id) ? <TopicIcon id={id} /> : <Server size={12} />}
            {nodeName(id)}
          </span>
        </Fragment>
      ))}
    </span>
  );
}

type SortKey = 'throughput' | 'p50' | 'p99' | 'hops';
const SORTS: Record<SortKey, (p: StreamPathway) => number> = { throughput: (p) => p.throughput, p50: (p) => p.latencyP50Ms, p99: (p) => p.latencyP99Ms, hops: (p) => p.hops };

/** The hops of a pathway: each topic → consumer edge on it. */
function hopsOf(p: StreamPathway, edges: StreamEdge[]): StreamEdge[] {
  const out: StreamEdge[] = [];
  for (let i = 1; i + 1 < p.nodes.length; i += 2) {
    const e = edges.find((x) => x.source === p.nodes[i] && x.target === p.nodes[i + 1]);
    if (e) out.push(e);
  }
  return out;
}

/** Data Streams → Pipelines: end-to-end pathways from the first producer to the last consumer. */
export function PipelinesPage() {
  const q = useStreamMap();
  const [params, setParams] = useSearchParams();
  const [filter, setFilter] = useState('');
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'throughput', desc: true });
  const open = params.get('pathway');
  const rows = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const list = (q.data?.pathways ?? []).filter((p) => !needle || p.nodes.some((n) => nodeName(n).toLowerCase().includes(needle)));
    const f = SORTS[sort.key];
    return [...list].sort((a, b) => (sort.desc ? f(b) - f(a) : f(a) - f(b)));
  }, [q.data, filter, sort]);
  if (!q.data) return null;
  const edges = q.data.edges;
  const head = (key: SortKey, label: string) => (
    <th className="num sortable" onClick={() => setSort((s) => ({ key, desc: s.key === key ? !s.desc : true }))} aria-sort={sort.key === key ? (sort.desc ? 'descending' : 'ascending') : undefined}>
      {label}
      {sort.key === key ? (sort.desc ? ' ↓' : ' ↑') : ''}
    </th>
  );
  const toggle = (id: string) => setParams((p) => (p.get('pathway') === id ? new URLSearchParams() : new URLSearchParams({ pathway: id })), { replace: true });
  return (
    <div className="dsm-page">
      <div className="dsm-toolbar">
        <label className="dsm-search">
          <Search size={14} />
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter pathways by service or topic" />
        </label>
        <span className="muted">
          {rows.length} of {q.data.pathways.length} pathways · latency adds up the hops from publish to consume
        </span>
      </div>
      {q.data.pathways.length === 0 ? (
        <Empty>No end-to-end pathways yet: consume spans need their publish span as parent or span link (context propagation through message headers).</Empty>
      ) : (
        <table className="dsm-table">
          <thead>
            <tr>
              <th />
              <th>PATHWAY</th>
              {head('hops', 'HOPS')}
              {head('p50', 'LATENCY P50')}
              {head('p99', 'LATENCY P99')}
              {head('throughput', 'THROUGHPUT')}
              <th>STATUS</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td colSpan={8} className="dsm-empty-row">
                  No matching results found
                </td>
              </tr>
            )}
            {rows.map((p) => {
              const expanded = open === p.id;
              return (
                <Fragment key={p.id}>
                  <tr className={`clickable${expanded ? ' selected' : ''}`} onClick={() => toggle(p.id)}>
                    <td className="dsm-caret">{expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</td>
                    <td>
                      <PathwayChain nodes={p.nodes} />
                    </td>
                    <td className="num">{p.hops}</td>
                    <td className="num">{fmtMs(p.latencyP50Ms)}</td>
                    <td className="num">{fmtMs(p.latencyP99Ms)}</td>
                    <td className="num">{fmtRate(p.throughput)}</td>
                    <td>{p.lagHigh ? <span className="dsm-pill bad">Consumer lag</span> : <span className="dsm-pill ok">Healthy</span>}</td>
                    <td>
                      <Link
                        className="btn btn-sm"
                        onClick={(e) => e.stopPropagation()}
                        to={`/data-streams/measure?start=${encodeURIComponent(p.nodes[0])}&end=${encodeURIComponent(p.nodes[p.nodes.length - 1])}`}
                      >
                        <Timer size={13} /> Measure
                      </Link>
                    </td>
                  </tr>
                  {expanded && (
                    <tr className="dsm-detail-row">
                      <td />
                      <td colSpan={7}>
                        <table className="dsm-table compact">
                          <thead>
                            <tr>
                              <th>HOP</th>
                              <th>CONSUMER GROUPS</th>
                              <th className="num">PUBLISH → CONSUME P50</th>
                              <th className="num">P99</th>
                              <th className="num">THROUGHPUT</th>
                              <th className="num">LAG</th>
                              <th className="num">LAG IN TIME</th>
                            </tr>
                          </thead>
                          <tbody>
                            {hopsOf(p, edges).map((e) => (
                              <tr key={`${e.source}>${e.target}`}>
                                <td>
                                  {nodeName(e.source)} → {nodeName(e.target)}
                                </td>
                                <td>{e.consumerGroups.join(', ') || '—'}</td>
                                <td className="num">{fmtMs(e.latencyP50Ms)}</td>
                                <td className="num">{fmtMs(e.latencyP99Ms)}</td>
                                <td className="num">{fmtRate(e.throughput)}</td>
                                <td className={`num${e.lagHigh ? ' dsm-bad' : ''}`}>{e.lag == null ? '—' : fmtCount(e.lag)}</td>
                                <td className={`num${e.lagHigh ? ' dsm-bad' : ''}`}>{fmtLagSeconds(e.lagSeconds)}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}

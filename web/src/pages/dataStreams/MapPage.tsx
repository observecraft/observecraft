import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Cylinder, Search, Server, Timer, X } from 'lucide-react';
import type { StreamMap, StreamNode } from '../../api/dataStreams/client';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { Empty } from '../../components/ui/AsyncState';
import { fmtCount, fmtLagSeconds, fmtMs, fmtRate, nodeName, systemLogo } from './format';
import { LineChart } from './LineChart';
import { PathwayChain } from './PipelinesPage';
import { useStreamMap, useStreamNode } from './queries';
import { StreamGraph, type EdgeLabel } from './StreamGraph';

/** APM trace search for a node: a service's spans, or the spans that publish to / consume a topic. */
export function tracesLink(n: StreamNode): string {
  const query = n.kind === 'topic' ? `@messaging.destination.name:${n.name}` : `service:${n.name}`;
  return `/apm/traces?query=${encodeURIComponent(query)}`;
}

/** Data Streams → Map: services and topics, the messages between them, and a panel for the selected node. */
export function MapPage() {
  const q = useStreamMap();
  const [params, setParams] = useSearchParams();
  const [filter, setFilter] = useState('');
  const selected = params.get('node');
  const edgeLabel: EdgeLabel = params.get('edges') === 'latency' ? 'latency' : 'throughput';
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const map = q.data;
  const dimmed = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle || !map) return undefined;
    return new Set(map.nodes.filter((n) => !n.name.toLowerCase().includes(needle)).map((n) => n.id));
  }, [filter, map]);
  if (!map) return null;
  const node = map.nodes.find((n) => n.id === selected) ?? null;
  const lagging = map.edges.filter((e) => e.lagHigh).length;
  return (
    <div className={`dsm-map-page${node ? ' with-panel' : ''}`}>
      <div className="dsm-map-main">
        <div className="dsm-toolbar">
          <label className="dsm-search">
            <Search size={14} />
            <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter services and topics" />
          </label>
          <div className="btn-group" role="group" aria-label="Edges show">
            <button className={`btn btn-sm${edgeLabel === 'throughput' ? ' btn-active' : ''}`} onClick={() => set('edges', null)}>
              Throughput
            </button>
            <button className={`btn btn-sm${edgeLabel === 'latency' ? ' btn-active' : ''}`} onClick={() => set('edges', 'latency')}>
              Latency
            </button>
          </div>
          <div className="dsm-legend">
            <span>
              <Server size={13} /> Service
            </span>
            <span>
              <Cylinder size={13} /> Topic / queue
            </span>
            <span className="dsm-legend-lag">High consumer lag{lagging ? ` (${lagging})` : ''}</span>
          </div>
        </div>
        {map.nodes.length === 0 ? (
          <Empty>No messaging spans in this range.</Empty>
        ) : (
          <div className="dsm-map-canvas">
            <StreamGraph nodes={map.nodes} edges={map.edges} selected={selected} onSelect={(id) => set('node', id)} edgeLabel={edgeLabel} dimmed={dimmed} />
          </div>
        )}
      </div>
      {node && <NodePanel node={node} map={map} onClose={() => set('node', null)} />}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: 'bad' }) {
  return (
    <div className={`dsm-stat${tone ? ` ${tone}` : ''}`}>
      <span>{label}</span>
      <b>{value}</b>
    </div>
  );
}

function NodePanel({ node, map, onClose }: { node: StreamNode; map: StreamMap; onClose: () => void }) {
  const detail = useStreamNode(node.id);
  const through = map.pathways.filter((p) => p.nodes.includes(node.id));
  const topic = node.kind === 'topic';
  // A topic's consumers, or the topics a service consumes.
  const consuming = map.edges.filter((e) => e.kind === 'consume' && (topic ? e.source === node.id : e.target === node.id));
  const d = detail.data;
  return (
    <aside className="dsm-panel" aria-label={`${node.name} details`}>
      <header>
        <div>
          <span className="dsm-kind">
            {topic && systemLogo(node.system) && <BrandLogo name={systemLogo(node.system)!} size={12} />} {topic ? `${node.system ?? 'queue'} topic` : 'service'}
          </span>
          <h2>{node.name}</h2>
        </div>
        <button className="icon-btn" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <div className="dsm-stats">
        <Stat label={topic ? 'Messages in' : 'Published'} value={fmtRate(node.produced)} />
        <Stat label={topic ? 'Delivered' : 'Consumed'} value={fmtRate(node.consumed)} />
        <Stat label="Consumer lag" value={node.lag == null ? '—' : `${fmtCount(node.lag)} msgs`} tone={node.lagHigh ? 'bad' : undefined} />
        <Stat label="Lag in time" value={fmtLagSeconds(node.lagSeconds)} tone={node.lagHigh ? 'bad' : undefined} />
      </div>
      <div className="dsm-panel-actions">
        <Link className="btn btn-sm" to={tracesLink(node)}>
          View traces
        </Link>
        {!topic && (
          <Link className="btn btn-sm" to={`/data-streams/measure?start=${encodeURIComponent(node.id)}`}>
            <Timer size={14} /> Measure from here
          </Link>
        )}
        {topic && node.system === 'kafka' && (
          <Link className="btn btn-sm" to={`/data-streams/kafka?tab=topics&q=${encodeURIComponent(node.name)}`}>
            Open in Kafka Console
          </Link>
        )}
      </div>
      <LineChart
        title={topic ? 'Messages' : 'Throughput'}
        fmt={fmtRate}
        series={[
          { name: topic ? 'In' : 'Published', points: d?.produced ?? [], color: '--chart-1' },
          { name: topic ? 'Delivered' : 'Consumed', points: d?.consumed ?? [], color: '--chart-3' },
        ]}
      />
      <LineChart title="Consumer lag (messages)" fmt={fmtCount} series={(d?.lag ?? []).map((l) => ({ name: topic ? l.group : `${l.group} · ${l.topic}`, points: l.points }))} />
      {consuming.length > 0 && (
        <section className="dsm-panel-section">
          <h3>{topic ? 'Consumers' : 'Consumed topics'}</h3>
          <table className="dsm-table compact">
            <thead>
              <tr>
                <th>{topic ? 'CONSUMER' : 'TOPIC'}</th>
                <th>GROUP</th>
                <th className="num">P50</th>
                <th className="num">P99</th>
                <th className="num">LAG</th>
              </tr>
            </thead>
            <tbody>
              {consuming.map((e) => (
                <tr key={`${e.source}>${e.target}`}>
                  <td>{nodeName(topic ? e.target : e.source)}</td>
                  <td>{e.consumerGroups.join(', ') || '—'}</td>
                  <td className="num">{fmtMs(e.latencyP50Ms)}</td>
                  <td className="num">{fmtMs(e.latencyP99Ms)}</td>
                  <td className={`num${e.lagHigh ? ' dsm-bad' : ''}`}>{e.lag == null ? '—' : fmtCount(e.lag)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
      <section className="dsm-panel-section">
        <h3>Pathways ({through.length})</h3>
        {through.length === 0 ? (
          <p className="muted">No end-to-end pathway goes through {node.name}: no consume span linked to a publish span here.</p>
        ) : (
          <ul className="dsm-panel-pathways">
            {through.slice(0, 8).map((p) => (
              <li key={p.id}>
                <Link to={`/data-streams/pipelines?pathway=${encodeURIComponent(p.id)}`}>
                  <PathwayChain nodes={p.nodes} highlight={node.id} />
                </Link>
                <span className="muted">
                  p50 {fmtMs(p.latencyP50Ms)} · {fmtRate(p.throughput)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </aside>
  );
}
